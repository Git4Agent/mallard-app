import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { renderToStaticMarkup } from "react-dom/server";
import {
  connectionDetail,
  connectionSuspends,
  draftSettingsFromForm,
  fieldsSet,
  findDestinationConflict,
  findStorageProvider,
  formFromDraft,
  secretOperationsFromForm,
  secretPresenceOf,
  storageConnectionIssue,
  storageDraftReady,
  storageSubtitle,
  storageSubtitleIsPath,
  StorageEditor,
  StorageLockedForm,
  StorageReconnectBand,
  type StorageEditorForm,
  applyFieldDerivations,
  suggestStorageName,
  storageNameIsAuto,
  storageNameForSettings,
} from "../../src/components/project-sync/StorageSettings";
import type { StorageSetupMode } from "../../src/components/project-sync/StorageSettings";
import { StorageDiscoveryPanel, type DiscoveryState } from "../../src/components/project-sync/StorageDiscovery";
import {
  CatalogStorageRepositoryRow,
  createStorageSetupMarker,
  loadPreparedR2Draft,
  reconcileStorageSetup,
  StorageRepositoryDetails,
  StorageRepositoryRow,
} from "../../src/components/project-sync/ProjectLinksWorkspace";
import {
  bucketNameIssue,
  idleR2State,
  R2SetupLane,
  WRANGLER_INSTALL_COMMAND,
  WRANGLER_INSTALL_AGENT_MESSAGE,
  r2SetupErrorCode,
  type R2SetupFlow,
  type R2SetupMode,
  type R2SetupState,
} from "../../src/components/project-sync/R2Setup";
import type {
  CatalogCapabilityDescriptor,
  CatalogStorageRepositoryView,
} from "../../src/components/storage-catalog/types";
import StorageIndexPage from "../../src/components/project-sync/StorageIndexPage";
import StorageProviderPicker from "../../src/components/project-sync/StorageProviderPicker";
import ProjectSidebar from "../../src/components/project-sync/ProjectSidebar";
import { shouldReloadConfigRevision } from "../../src/components/project-sync/ProjectSync";
import { StorageDraftSaveQueue } from "../../src/components/project-sync/storageDraftSaveQueue";
import { iconNameFromHint } from "../../src/components/Icons";
import { storageAccessErrorMessage } from "../../src/storageError";
import type {
  RemoteBundleSummary,
  StorageConnectionHealth,
  StorageDiscoveryResult,
  StorageProviderDescriptor,
  StorageSetupDraft,
  StorageSummary,
} from "../../src/types";

/* Two fake descriptors stand in for whatever the backend registry compiles:
   a credentialed provider with a declared assistant, and a folder provider
   with one directory field. Nothing under test knows either by name. */
const cloudProvider: StorageProviderDescriptor = {
  key: "cloud-a",
  config_version: 1,
  display_name: "Cloud A",
  description: "A bucket reached through its object API.",
  display_rank: 10,
  icon: "cloud",
  clear_mode: "unconditional",
  fields: [
    { field_id: "api_url", label: "API URL", kind: "url", required: true, placeholder: "https://example.test/bucket", help: "Copy it from the provider console." },
    { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true },
    { field_id: "region_hint", label: "Region hint", kind: "text", required: false },
  ],
  secrets: [
    { secret_id: "api_secret", label: "API Secret", required: true, help: "Shown once when the key is created." },
  ],
  managed_secret_id: null,
  local_setup: null,
};

const folderProvider: StorageProviderDescriptor = {
  key: "local",
  config_version: 1,
  display_name: "Local folder",
  description: "A folder on this Mac or on a mounted drive.",
  display_rank: 20,
  icon: "drive",
  clear_mode: "revision_checked",
  fields: [
    { field_id: "directory", label: "Folder", kind: "directory", required: true, help: "An absolute path on this Mac." },
  ],
  secrets: [],
  managed_secret_id: null,
};

const emptyForm: StorageEditorForm = { name: "", settings: {}, typedSecrets: {} };

function folderSummary(patch: Partial<StorageSummary> = {}): StorageSummary {
  return {
    storage_id: "storage-local",
    name: "Local storage 1",
    provider: "local",
    config_version: 1,
    provider_label: "Local folder",
    icon: "drive",
    available: true,
    summary: [{ field_id: "directory", label: "Folder", value: "/Users/test/storage", is_path: true }],
    settings: { directory: "/Users/test/storage" },
    secrets: [],
    ...patch,
  };
}

function cloudSummary(patch: Partial<StorageSummary> = {}): StorageSummary {
  return {
    storage_id: "storage-cloud",
    name: "Cloud storage",
    provider: "cloud-a",
    config_version: 1,
    provider_label: "Cloud A",
    icon: "cloud",
    available: true,
    summary: [
      { field_id: "bucket", label: "Bucket", value: "mallard-team", is_path: false },
      { field_id: "account_id", label: "Account ID", value: "0123456789abcdef0123456789abcdef", is_path: false },
    ],
    settings: { api_url: "https://example.test/team", access_key_id: "key-id" },
    secrets: [{ secret_id: "api_secret", present: true }],
    ...patch,
  };
}

function cloudDraft(patch: Partial<StorageSetupDraft> = {}): StorageSetupDraft {
  return {
    draft_id: "storagedraft-0123456789abcdef01234567",
    storage_id: "storage-drafted",
    name: "Team backup",
    provider: "cloud-a",
    config_version: 1,
    provider_label: "Cloud A",
    available: true,
    settings: { api_url: "https://example.test/team-backup", access_key_id: "prepared-access-key" },
    secrets: [{ secret_id: "api_secret", present: true }],
    edits_existing: false,
    revision: 1,
    created_at: 1,
    updated_at: 1,
    ...patch,
  };
}

test("the editor renders the fields and secrets its descriptor declares", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      descriptor={cloudProvider}
      form={emptyForm}
      secretPresence={{}}
      disabled={false}
      onChange={() => undefined}
      setupSlot={<button type="button">Run with Claude Code</button>}
    />,
  );

  // The header names the storage and carries the type switch, listing every
  // provider the build offers with the draft's own checked; a new draft may
  // restart on another type through it.
  assert.match(html, /aria-label="Storage name"/);
  assert.match(html, /data-icon="cloud"/);
  assert.match(html, /class="sync-segmented" role="radiogroup" aria-label="Storage type"/);
  assert.match(html, /role="radio" aria-checked="true" class="active"[^>]*>Cloud A</);
  assert.doesNotMatch(html, /sync-storage-editor-provider/);
  assert.doesNotMatch(html, /Set up with an agent|Run with Claude Code/);
  // Fields in declaration order, typed by kind; help rides the ? mark.
  assert.match(html, /API URL[\s\S]*Access Key ID[\s\S]*Region hint[\s\S]*API Secret/);
  assert.match(html, /<input type="url"[^>]*placeholder="https:\/\/example\.test\/bucket"/);
  assert.match(html, /<input type="text"[^>]*required=""/);
  assert.equal((html.match(/class="field-help"/g) ?? []).length, 2);
  assert.match(html, /aria-label="Copy it from the provider console\."/);
  assert.match(html, /class="storage-provider-field storage-provider-url-field"/);
  // The secret is a password field with a reveal; nothing stored is shown.
  assert.match(html, /type="password"/);
  assert.match(html, /sync-secret-reveal/);
  assert.match(html, />Show</);
  assert.doesNotMatch(html, /Stored — type to replace|readonly|Bucket|Account ID|Create storage|Finish setup/);
});

test("a folder provider renders its directory field with Browse and no credential rows", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      descriptor={folderProvider}
      form={{ ...emptyForm, name: "Local storage 1", settings: { directory: "/tmp/storage" } }}
      secretPresence={{}}
      disabled={false}
      onChange={() => undefined}
    />,
  );

  assert.match(html, /<label class="sync-folder-field compact"><span class="sync-field-name">Folder<\/span><div><input[^>]*\/tmp\/storage[^>]*><button[^>]*>.*Browse…<\/button><\/div><small>An absolute path on this Mac\.<\/small><\/label>/);
  assert.match(html, /role="radio" aria-checked="true" class="active"[^>]*>Local folder</);
  assert.doesNotMatch(html, /storage-setup-route-row|sync-storage-fields|type="password"|Manual setup/);
});

test("a folder another storage already occupies is found as typed, not as canonicalised", () => {
  // Same provider, same directory field, same folder up to a trailing slash:
  // the page catches what it can read off a storage's summary facts. Symlinks
  // and case stay the backend's call — it refuses the same folder on Test
  // and Save through its canonical check.
  const owner = folderSummary({ storage_id: "storage-owner", name: "Backup drive", summary: [
    { field_id: "directory", label: "Folder", value: "/Volumes/Backup/mallard/", is_path: true },
  ] });
  const others = [cloudSummary(), folderSummary({ storage_id: "storage-elsewhere" }), owner];
  const typed = (directory: string) => ({ settings: { directory } });
  assert.equal(findDestinationConflict(folderProvider, typed("/Volumes/Backup/mallard"), others, null), owner);
  assert.equal(findDestinationConflict(folderProvider, typed("  /Volumes/Backup/mallard//  "), others, null), owner);
  // The storage being edited is not a conflict with itself.
  assert.equal(findDestinationConflict(folderProvider, typed("/Volumes/Backup/mallard"), others, "storage-owner"), null);
  // A different folder, an empty field, and a bare root all pass.
  assert.equal(findDestinationConflict(folderProvider, typed("/Volumes/Backup/mallard2"), others, null), null);
  assert.equal(findDestinationConflict(folderProvider, typed(""), others, null), null);
  assert.equal(findDestinationConflict(folderProvider, typed("/"), others, null), null);
  // Another provider's storage never owns a folder, and a non-path fact is
  // never compared, even when its text happens to match.
  const lookalike = cloudSummary({ summary: [{ field_id: "directory", label: "Folder", value: "/Volumes/Backup/mallard", is_path: false }] });
  assert.equal(findDestinationConflict(folderProvider, typed("/Volumes/Backup/mallard"), [lookalike], null), null);
  assert.equal(findDestinationConflict(cloudProvider, { settings: { api_url: "/Volumes/Backup/mallard" } }, others, null), null);
});

test("a taken folder is refused under the field, with a door to the storage that owns it", () => {
  const owner = folderSummary({ storage_id: "storage-owner", name: "Backup drive" });
  const render = (conflict: boolean) => renderToStaticMarkup(
    <StorageEditor
      descriptor={folderProvider}
      form={{ ...emptyForm, name: "Local storage 2", settings: { directory: "/Users/test/storage" } }}
      secretPresence={{}}
      disabled={false}
      onChange={() => undefined}
      destinationConflict={conflict ? { storage: owner, onOpen: () => undefined } : null}
    />,
  );
  const refused = render(true);
  assert.match(refused, /class="notice danger storage-destination-conflict" role="alert"/);
  assert.match(refused, /This folder is already connected as Backup drive\./);
  assert.match(refused, /class="btn btn-primary"[^>]*>Open Backup drive</);
  assert.match(refused, /Choose another…/);
  // The band sits after the folder field, inside the card, so the field it
  // refers to is the one directly above it.
  assert.ok(refused.indexOf("sync-folder-field") < refused.indexOf("storage-destination-conflict"));
  assert.doesNotMatch(render(false), /storage-destination-conflict|already connected/);

  // The verbs live in the workspace, which gates all four — Test, Save,
  // Create, and the create form's Test — on the same blocked flag.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /const storageActionBlocked = formLocked \|\| !storageReady \|\| destinationConflict !== null;/);
  assert.equal(workspace.match(/disabled=\{storageActionBlocked\}/g)?.length, 4);
  assert.doesNotMatch(workspace, /disabled=\{formLocked \|\| !storageReady\}/);
});

test("a settled storage opens as the locked form: read-only fields, a masked secret with Show and Copy, and the name as the one live field", () => {
  const revealed: string[] = [];
  const html = renderToStaticMarkup(
    <StorageLockedForm
      storage={cloudSummary()}
      descriptor={cloudProvider}
      disabled={false}
      onRename={async () => true}
      onRevealSecret={async (secretId) => { revealed.push(secretId); return "never-rendered"; }}
      onTest={() => undefined}
      testing={false}
      verifiedAt={null}
      reconnect={{ open: false, onOpen: () => undefined, band: null }}
    />,
  );
  // The header keeps the name field and states the type as a label: no
  // type switch, no trash, nothing that says Save.
  assert.match(html, /class="sync-storage-card sync-storage-locked"/);
  assert.match(html, /<form class="sync-storage-name"><input[^>]*aria-label="Storage name"[^>]*value="Cloud storage"/);
  assert.match(html, /sync-storage-editor-provider">Cloud A</);
  // Unchanged, the name shows no verbs; typing makes Save and Cancel appear
  // beside it (the project rename's pair), Enter submits, Escape cancels.
  assert.doesNotMatch(html, /sync-storage-name-verbs/);
  const form = readFileSync("src/components/project-sync/StorageSettings.tsx", "utf8");
  assert.match(form, /\{nameDirty && \(\s*<span className="sync-storage-name-verbs">\s*<button type="submit" className="btn btn-primary"[\s\S]*?Save[\s\S]*?<button type="button" className="btn btn-ghost"[^>]*onClick=\{cancelName\}>\s*Cancel/);
  assert.doesNotMatch(form, /onBlur=\{\(\) => void commitName\(\)\}/);
  assert.doesNotMatch(html, /sync-segmented|Save storage|Remove storage|storage-setup-route-row/);
  // Every declared field is readonly with its saved value and a lock; never disabled.
  assert.match(html, /<input type="url"[^>]*readOnly=""[^>]*value="https:\/\/example\.test\/team"/);
  assert.match(html, /<input type="text"[^>]*readOnly=""[^>]*value="key-id"/);
  assert.equal((html.match(/class="sync-storage-lock"/g) ?? []).length, 4, "three fields and the secret each wear the lock");
  assert.doesNotMatch(html, /disabled=""/);
  // The secret is a masked readonly field: the saved value is not in the page.
  const secretInput = html.match(/<input type="password"[^>]*>/)?.[0] ?? "";
  assert.match(secretInput, /readOnly=""/);
  assert.match(secretInput, /value="•+"/);
  assert.doesNotMatch(html, /never-rendered|Stored — type to replace/);
  assert.deepEqual(revealed, [], "nothing is fetched on mount");
  assert.match(html, /class="sync-secret-copy"[^>]*aria-label="Copy API Secret"/);
  assert.match(html, /sync-secret-reveal" aria-pressed="false">Show</);
  // The guide row says what this visit knows; the footer states the rule and keeps Test.
  assert.match(html, /role="note"[\s\S]*?Configured\. Test the connection to verify it\./);
  assert.match(html, /sync-storage-fixed">[\s\S]*?destination and credentials are fixed\. To sync somewhere else, add a new storage\./);
  assert.match(html, /<button type="button" class="btn">Test connection<\/button>/);
  assert.doesNotMatch(html, /Reconnect…|sync-storage-refused-chip/);

  // A secret the storage does not hold has no verbs and says so.
  const missing = renderToStaticMarkup(
    <StorageLockedForm
      storage={cloudSummary({ secrets: [{ secret_id: "api_secret", present: false }] })}
      descriptor={cloudProvider}
      disabled={false}
      onRename={async () => true}
      onRevealSecret={async () => ""}
      onTest={() => undefined}
      testing={false}
      verifiedAt={null}
      reconnect={{ open: false, onOpen: () => undefined, band: null }}
    />,
  );
  assert.match(missing, /placeholder="Not stored"/);
  assert.doesNotMatch(missing, /sync-secret-copy|>Show</);

  // Verified this visit: the row says so and the footer wears the chip.
  const verified = renderToStaticMarkup(
    <StorageLockedForm
      storage={cloudSummary()}
      descriptor={cloudProvider}
      disabled={false}
      onRename={async () => true}
      onRevealSecret={async () => ""}
      onTest={() => undefined}
      testing={false}
      verifiedAt={Date.now() - 5_000}
      reconnect={{ open: false, onOpen: () => undefined, band: null }}
    />,
  );
  assert.match(verified, /data-icon="check-circle"[\s\S]*?Connected · verified/);
  assert.match(verified, /storage-setup-verified[^>]*>[\s\S]*?Connection verified/);

  // The workspace reads the page off the listing and asks the backend only
  // on Show or Copy, through the one reveal door.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /onRevealSecret=\{\(secretId\) => projectSyncApi\.revealStorageSecret\(pageStorageId, secretId\)\}/);
  assert.doesNotMatch(workspace, /Save storage|settingsExpanded\) void ensureStorageEditSession/);
  // One storage page: the card is unconditional, over the repositories.
  assert.match(workspace, /\{!creatingStorage && savedEditedStorage && descriptor && \(\s*<StorageLockedForm/);
  assert.doesNotMatch(workspace, /storageDetailsExpanded|Hide storage settings|label: "Repositories"/);
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.match(api, /invoke<string>\("reveal_storage_secret", \{ storageId, secretId \}\)/);
  assert.match(api, /invoke<TestStorageResult>\("test_storage", \{ storageId \}\)/);
});

test("readiness and the tally count required fields plus required secrets, stored or typed", () => {
  const partial: StorageEditorForm = {
    name: "Team backup",
    settings: { api_url: "https://example.test/team", access_key_id: "key-id" },
    typedSecrets: {},
  };
  assert.deepEqual(fieldsSet(cloudProvider, emptyForm, {}), { set: 0, total: 3 });
  assert.deepEqual(fieldsSet(cloudProvider, partial, {}), { set: 2, total: 3 });
  assert.equal(storageDraftReady(cloudProvider, partial, {}), false);
  // A typed secret and a stored one count the same; the optional field never counts.
  assert.deepEqual(fieldsSet(cloudProvider, { ...partial, typedSecrets: { api_secret: "typed" } }, {}), { set: 3, total: 3 });
  assert.equal(storageDraftReady(cloudProvider, { ...partial, typedSecrets: { api_secret: "typed" } }, {}), true);
  assert.deepEqual(fieldsSet(cloudProvider, partial, { api_secret: true }), { set: 3, total: 3 });
  assert.equal(storageDraftReady(cloudProvider, partial, { api_secret: true }), true);
  assert.equal(storageDraftReady(cloudProvider, { ...partial, settings: { ...partial.settings, region_hint: "eu" } }, {}), false);
  // The name gates readiness without joining the tally; whitespace is not a value.
  assert.equal(storageDraftReady(cloudProvider, { ...partial, name: "  ", typedSecrets: { api_secret: "typed" } }, {}), false);
  assert.equal(storageDraftReady(cloudProvider, { ...partial, settings: { ...partial.settings, access_key_id: " " }, typedSecrets: { api_secret: "typed" } }, {}), false);
  // A folder needs its one path and nothing else.
  assert.deepEqual(fieldsSet(folderProvider, emptyForm, {}), { set: 0, total: 1 });
  assert.equal(storageDraftReady(folderProvider, { ...emptyForm, name: "Local", settings: { directory: "/tmp/storage" } }, {}), true);
});

test("the form is built from the draft and sent back as settings plus secret operations", () => {
  const draft = cloudDraft({ settings: { api_url: "https://example.test/team", access_key_id: "key-id", probe_result: true } });
  const form = formFromDraft(cloudProvider, draft);
  assert.deepEqual(form, {
    name: "Team backup",
    settings: { api_url: "https://example.test/team", access_key_id: "key-id", region_hint: "" },
    typedSecrets: {},
  });
  assert.deepEqual(secretPresenceOf(draft), { api_secret: true });

  // Declared fields travel trimmed, over whatever else the adapter keeps in
  // the draft; the form never drops a key it does not show.
  assert.deepEqual(
    draftSettingsFromForm(cloudProvider, { ...form, settings: { ...form.settings, access_key_id: " key-id-2 " } }, draft.settings),
    { api_url: "https://example.test/team", access_key_id: "key-id-2", region_hint: "", probe_result: true },
  );
  // Only touched secrets travel: replace over a stored one, set over a
  // missing one, and an untouched or emptied field means unchanged.
  assert.deepEqual(secretOperationsFromForm({ typedSecrets: { api_secret: "new" } }, { api_secret: true }), {
    api_secret: { op: "replace", value: "new" },
  });
  assert.deepEqual(secretOperationsFromForm({ typedSecrets: { api_secret: "new" } }, {}), {
    api_secret: { op: "set", value: "new" },
  });
  assert.deepEqual(secretOperationsFromForm({
    typedSecrets: { api_secret: "new" },
    persistedSecrets: { api_secret: "new" },
  }, { api_secret: true }), {}, "an unchanged persisted value does not travel again");
  assert.deepEqual(secretOperationsFromForm({
    typedSecrets: { api_secret: "newer" },
    persistedSecrets: { api_secret: "new" },
  }, { api_secret: true }), {
    api_secret: { op: "replace", value: "newer" },
  });
  assert.deepEqual(secretOperationsFromForm({ typedSecrets: { api_secret: "" } }, { api_secret: true }), {});
  assert.deepEqual(secretOperationsFromForm({ typedSecrets: {} }, { api_secret: true }), {});

  assert.equal(findStorageProvider([folderProvider, cloudProvider], "cloud-a", 1), cloudProvider);
  assert.equal(findStorageProvider([folderProvider, cloudProvider], "cloud-a", 2), null);
  assert.equal(findStorageProvider([folderProvider, cloudProvider], "dropbox", 1), null);
});

test("the subtitle prefers the path fact, then the first fact, then the provider label", () => {
  assert.equal(storageSubtitle(folderSummary()), "~/storage");
  assert.equal(storageSubtitleIsPath(folderSummary()), true);
  assert.equal(storageSubtitle(cloudSummary()), "mallard-team");
  assert.equal(storageSubtitleIsPath(cloudSummary()), false);
  // A path fact wins wherever the adapter lists it.
  assert.equal(storageSubtitle(cloudSummary({ summary: [
    { field_id: "bucket", label: "Bucket", value: "mallard-team", is_path: false },
    { field_id: "cache", label: "Cache", value: "/Users/test/cache", is_path: true },
  ] })), "~/cache");
  // A record this build cannot read has no facts; its provider label stands in.
  assert.equal(storageSubtitle(cloudSummary({ available: false, provider_label: "dropbox", summary: [] })), "dropbox");
  assert.equal(storageSubtitleIsPath(cloudSummary({ summary: [] })), false);
});

test("connection failures use actionable storage language named after the provider", () => {
  assert.equal(
    storageAccessErrorMessage(cloudSummary(), "S3 get '.mallard/_storage.json': dispatch failure"),
    "Couldn’t reach Cloud A. Check the network and try again.",
  );
  // The first storage write a new user makes fails behind a prefix.
  assert.equal(
    storageAccessErrorMessage(cloudSummary(), "register the repo in 'Cloud storage': S3 conditional put 'bundles/x': dispatch failure: io error"),
    "Couldn’t reach Cloud A. Check the network and try again.",
  );
  assert.equal(
    storageAccessErrorMessage(cloudSummary(), "S3 get 'x': service error: unhandled error (AccessDenied)"),
    "Cloud A rejected the credentials. Reconnect the storage with a current key, then try again.",
  );
  assert.equal(
    storageAccessErrorMessage(cloudSummary(), "S3 list 'bundles/': service error: NoSuchBucket"),
    "The Cloud A bucket can’t be found. Check the bucket name in the storage settings.",
  );
  assert.equal(
    storageAccessErrorMessage(cloudSummary(), "S3 get 'x': service error: unhandled error"),
    "Couldn’t connect to Cloud A. Check the connection settings and credentials, then try again.",
  );
  assert.equal(
    storageAccessErrorMessage(cloudSummary(), "remote bundle cursor did not advance"),
    "remote repo cursor did not advance",
  );
  assert.equal(
    storageAccessErrorMessage(null, "S3 get '.mallard/_storage.json': dispatch failure"),
    "S3 get '.mallard/_storage.json': dispatch failure",
  );
});

test("icon hints resolve to a mark, and an unknown hint takes the caller's fallback", () => {
  assert.equal(iconNameFromHint("cloud", "drive"), "cloud");
  assert.equal(iconNameFromHint("drive", "cloud"), "drive");
  assert.equal(iconNameFromHint("storage", "drive"), "drive");
  assert.equal(iconNameFromHint(undefined, "drive"), "drive");
  assert.equal(iconNameFromHint("", "cloud"), "cloud");
});

test("setup reconciliation adopts only a newer revision of the exact draft", () => {
  const saved = cloudDraft({ revision: 4, name: " Team backup ", settings: {}, secrets: [{ secret_id: "api_secret", present: false }] });
  const marker = createStorageSetupMarker(saved, saved.name);
  assert.deepEqual(marker, {
    draftId: saved.draft_id,
    revision: 4,
    storageId: "storage-drafted",
    requestedName: "Team backup",
  });
  // The marker is built from the draft's identity alone; a typed secret
  // never enters it.
  assert.equal(JSON.stringify(marker).includes("canary"), false);

  const prepared = cloudDraft({ revision: 6, name: "Team backup" });
  const sameNameOtherDraft = cloudDraft({ draft_id: "storagedraft-other", revision: 50 });
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, [sameNameOtherDraft, prepared]), {
    kind: "adopt",
    draft: prepared,
  });
  // Only the exact draft, only a newer revision.
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, []), { kind: "preserve", reason: "no_update" });
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, [cloudDraft({ revision: 4 })]), { kind: "preserve", reason: "no_update" });
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, [sameNameOtherDraft]), { kind: "preserve", reason: "no_update" });
  // A newer revision that changed identity, was renamed, or is incomplete
  // is refused rather than adopted.
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, [cloudDraft({ revision: 5, storage_id: "storage-changed" })]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, [cloudDraft({ revision: 5, name: "Renamed" })]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, [cloudDraft({ revision: 5, secrets: [{ secret_id: "api_secret", present: false }] })]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileStorageSetup(marker, cloudProvider, [cloudDraft({ revision: 5, settings: { api_url: "" } })]), { kind: "preserve", reason: "invalid" });
});

test("config revision events recover invalid startup snapshots without reloading valid peers", () => {
  assert.equal(shouldReloadConfigRevision(5, 4, true), false);
  assert.equal(shouldReloadConfigRevision(5, 5, true), false);
  assert.equal(shouldReloadConfigRevision(5, 6, true), true);
  assert.equal(shouldReloadConfigRevision(0, 0, false), true);
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
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Version<\/span><b>3<\/b>/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Updated<\/span><b>29m ago<\/b>/);
  assert.doesNotMatch(html, /<span>Rev<\/span>/);
  assert.doesNotMatch(html, /<span>Upd<\/span>/);
  // The id lives in the fold now: no stub on the collapsed row, and the fold
  // itself is closed until the history table's own toggle opens it.
  assert.doesNotMatch(html, /440ed684…a8f0ea/);
  assert.match(html, /sync-history-commit-toggle/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /aria-controls="repo-details-440ed684dbb6034cf32c3bdf04a8f0ea"/);
  assert.match(html, /aria-label="Show details for mallardInternal"/);
});

/* The repository DTO carries its coordinate and the backend's labels; the
   storage page joins it to the capability the coordinate registers, which is
   where the glyph and the capability's own label come from. */
function catalogCapability(
  resourceKind: string,
  format: string,
  formatVersion: number,
  label: string,
  singularLabel: string,
  icon: string,
  kindLabel: string,
  formatLabel: string,
): CatalogCapabilityDescriptor {
  return {
    provider: "codex",
    provider_label: "Codex",
    resource_kind: resourceKind,
    format,
    format_version: formatVersion,
    label,
    singular_label: singularLabel,
    presentation: { icon, kind_label: kindLabel, format_label: formatLabel },
    capture: "directory_tree",
    apply: "automatic",
    targets: ["provider-home.skills"],
  };
}

test("Catalog kind repositories join the listing in the project row's grammar", () => {
  const capability = catalogCapability("agent_skill", "agentskills.io.skill", 1, "Skills", "Skill", "skill", "Skills", "Agent Skill");
  const repo: CatalogStorageRepositoryView = {
    repository_id: "library-3a91f04c77d2b8e6a5c31f9d04b7e28c",
    provider: capability.provider,
    resource_kind: capability.resource_kind,
    format: capability.format,
    format_version: capability.format_version,
    provider_label: capability.provider_label,
    kind_label: capability.presentation.kind_label,
    format_label: capability.presentation.format_label,
    revision: 10,
    updated_at: Math.floor(Date.now() / 1000) - 29 * 60,
    resource_names: ["changelog-writer", "deploy-checklist", "triage"],
    error: null,
    capability,
  };
  const html = renderToStaticMarkup(<CatalogStorageRepositoryRow repo={repo} />);

  // Same item, row, chips, and fold affordance as a project repository; the
  // identity is the kind icon and the scope-label name one level up.
  assert.match(html, /sync-storage-repository-item/);
  assert.match(html, /sync-storage-repository-row/);
  assert.match(html, /Skills · Codex/);
  assert.match(html, /data-icon="lightbulb"/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Version<\/span><b>10<\/b>/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Updated<\/span><b>29m ago<\/b>/);
  assert.match(html, /sync-history-commit-toggle/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /aria-controls="catalog-repo-details-library-3a91f04c77d2b8e6a5c31f9d04b7e28c"/);
  // The fold is closed until opened; no id stub or contents on the row.
  assert.doesNotMatch(html, /changelog-writer/);
  assert.doesNotMatch(html, /Can’t be read/);

});

test("an unreadable Catalog repository states its refusal instead of chips", () => {
  const capability = catalogCapability("desktop_pet", "openai.codex-pet", 1, "Pets v1", "Pet v1", "pet", "Pets", "Codex pet v1");
  const repo: CatalogStorageRepositoryView = {
    repository_id: "library-e29a51c8d40b76f3a1c58e02b9d47f61",
    provider: capability.provider,
    resource_kind: capability.resource_kind,
    format: capability.format,
    format_version: capability.format_version,
    provider_label: capability.provider_label,
    kind_label: capability.presentation.kind_label,
    format_label: capability.presentation.format_label,
    revision: null,
    updated_at: null,
    resource_names: [],
    error: "library tag does not match its provider and kind derivation",
    capability,
  };
  const html = renderToStaticMarkup(<CatalogStorageRepositoryRow repo={repo} />);

  assert.match(html, /Pets v1 · Codex/);
  assert.match(html, /sync-storage-repo-unreadable[^>]*>Can’t be read/);
  assert.match(html, /sync-storage-repository-icon danger/);
  // No chips on a refusal row — the phrase takes their place — and the
  // backend's sentence waits in the fold.
  assert.doesNotMatch(html, /<span>Version<\/span>/);
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
        folderSummary(),
        cloudSummary(),
        cloudSummary({ storage_id: "storage-future", name: "Future storage", provider: "dropbox", provider_label: "dropbox", icon: "storage", available: false, summary: [] }),
      ]}
      links={[
        { local_project_id: "p1", storage_id: "storage-local", state: "active", revision: 1, updated_at: 1 },
        { local_project_id: "p2", storage_id: "storage-local", state: "active", revision: 1, updated_at: 1 },
      ] as never}
      storageIssues={{ "storage-cloud": "Bucket unreachable" }}
      drafts={[
        cloudDraft({ name: "Half-typed cloud" }),
        cloudDraft({ draft_id: "storagedraft-edit", storage_id: "storage-cloud", name: "Cloud storage", edits_existing: true }),
      ]}
      storageProviders={[cloudProvider, folderProvider]}
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

  // Cards: name, location from the summary, the icon hint, the linked-project
  // tally; the issue marks its card and the accessible name carries it whole.
  assert.match(html, /aria-label="Open Local storage 1"[\s\S]*?data-icon="drive"[\s\S]*?<strong>Local storage 1<\/strong>[\s\S]*?landing-picker-card-location">~\/storage</);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Projects<\/span><b>2<\/b>/);
  assert.match(html, /aria-label="Open Cloud storage\. Storage issue: Bucket unreachable"[\s\S]*?data-icon="cloud"[\s\S]*?landing-picker-card-location">mallard-team</);
  assert.match(html, /sync-storage-index-warning[^>]*>[\s\S]*?<span>Issue<\/span>/);
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Projects<\/span><b>0<\/b>/);
  // A record from a newer build keeps its card: listed, named by its raw
  // provider key, flagged as unavailable, drawn with the fallback mark.
  assert.match(html, /aria-label="Open Future storage\. Unavailable: this build has no dropbox adapter"/);
  assert.match(html, /<span>Unavailable<\/span>/);
  assert.match(html, /landing-picker-card-location">dropbox</);

  // Drafts are cards too: badged, tally-less, opening their draft, wearing
  // their provider's icon. A new draft says it is unfinished; an edit draft
  // says whose settings it holds. No secret ever reaches the page.
  assert.match(html, /aria-label="Continue setting up Half-typed cloud — resumable storage draft"[\s\S]*?data-icon="cloud"/);
  assert.match(html, /<strong>Half-typed cloud<\/strong>[\s\S]*?landing-picker-card-location">Setup not finished — not connected to Mallard yet</);
  assert.match(html, /landing-picker-card-location">Reconnect not finished — Cloud A credential not saved yet</);
  assert.equal((html.match(/sync-draft-badge">Draft/g) ?? []).length, 2);
  assert.equal((html.match(/sync-storage-repo-chip/g) ?? []).length, 3);
  assert.doesNotMatch(html, /prepared-access-key|api_secret|value="/);
});

test("the Storages index, empty, is the filled page ghosted: one door per provider, no explainer", () => {
  const chosen: Array<string | undefined> = [];
  const html = renderToStaticMarkup(
    <StorageIndexPage
      storages={[]}
      links={[] as never}
      storageIssues={{}}
      drafts={[]}
      storageProviders={[cloudProvider, folderProvider]}
      busy={false}
      appActions={[]}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onAddStorage={(provider) => chosen.push(provider)}
    />,
  );

  // The same head as the filled page, with the count at zero and the bare
  // verb, which still goes through the picker.
  assert.match(html, /landing-picker-head[\s\S]*?0 on this machine[\s\S]*?class="btn btn-primary"[^>]*>[\s\S]*?Add storage…/);
  // One dashed door per provider the build ships, in the backend's order,
  // wearing the provider's hint; a door names its provider on the way in.
  const doors = [...html.matchAll(/class="landing-picker-door" aria-label="([^"]+)"/g)].map((match) => match[1]);
  assert.deepEqual(doors, ["Add Cloud A storage", "Add Local folder storage"]);
  assert.match(html, /aria-label="Add Cloud A storage"[\s\S]*?data-icon="cloud"[\s\S]*?<strong>Cloud A<\/strong>[\s\S]*?data-icon="plus"/);
  assert.match(html, /aria-label="Add Local folder storage"[\s\S]*?data-icon="drive"/);
  assert.match(html, /landing-picker-caption"[\s\S]*?data-icon="folder"[\s\S]*?data-icon="catalog"[\s\S]*?Projects &amp; Catalog sync here/);
  assert.doesNotMatch(html, /landing-picker-empty|No storages yet|mallard-logo-duck/);
  assert.doesNotMatch(html, /Add a storage to choose|landing-picker-card"/);

  // Before the descriptors arrive, the one door is the picker itself.
  const unanswered = renderToStaticMarkup(
    <StorageIndexPage
      storages={[]}
      links={[] as never}
      storageIssues={{}}
      drafts={[]}
      storageProviders={[]}
      busy={false}
      appActions={[]}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onAddStorage={() => undefined}
    />,
  );
  assert.equal((unanswered.match(/class="landing-picker-door"/g) ?? []).length, 1);
  assert.match(unanswered, /class="landing-picker-door" aria-label="Add storage"[\s\S]*?data-icon="storage"/);

  // A draft alone is content: the empty state yields to its row, and a draft
  // without a descriptor in this build still draws a mark.
  const draftsOnly = renderToStaticMarkup(
    <StorageIndexPage
      storages={[]}
      links={[] as never}
      storageIssues={{}}
      drafts={[cloudDraft({ name: "", provider: "dropbox", provider_label: "dropbox" })]}
      storageProviders={[]}
      busy={false}
      appActions={[]}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onAddStorage={() => undefined}
    />,
  );
  assert.doesNotMatch(draftsOnly, /landing-picker-empty/);
  assert.match(draftsOnly, /<strong>\(unnamed\)<\/strong>/);
  assert.match(draftsOnly, /data-icon="drive"/);
});

test("Add storage starts with a provider choice drawn from the backend's descriptors", () => {
  const html = renderToStaticMarkup(
    <StorageProviderPicker
      providers={[cloudProvider, folderProvider]}
      busy={false}
      appActions={[]}
      onChoose={() => undefined}
      onCancel={() => undefined}
    />,
  );

  // Rooted at the Storages crumb like every storage screen; the leaf says
  // what the page makes.
  assert.match(html, /aria-label="New storage path"/);
  assert.match(html, /All storages/);
  assert.match(html, /sync-path-crumb[^>]*leaf[^>]*>[\s\S]*?New storage/);
  assert.match(html, /Choose a storage type/);
  // One door per descriptor, in the order the backend listed them, each
  // carrying its icon hint, name, and description — no fixed provider list.
  assert.equal((html.match(/class="landing-picker-card"/g) ?? []).length, 2);
  assert.match(html, /aria-label="Set up Cloud A storage"[\s\S]*?data-icon="cloud"[\s\S]*?<strong>Cloud A<\/strong>[\s\S]*?A bucket reached through its object API\./);
  assert.match(html, /aria-label="Set up Local folder storage"[\s\S]*?data-icon="drive"[\s\S]*?<strong>Local folder<\/strong>/);
  assert.ok(html.indexOf("Cloud A") < html.indexOf("Local folder"));
  assert.match(html, /class="btn btn-ghost"[^>]*>Cancel/);

  const empty = renderToStaticMarkup(
    <StorageProviderPicker providers={[]} busy={false} appActions={[]} onChoose={() => undefined} onCancel={() => undefined} />,
  );
  assert.match(empty, /No storage providers in this build/);
  assert.doesNotMatch(empty, /landing-picker-card"/);

});

test("storage draft saves are serial and failed navigation keeps the form", async () => {
  const deferred = <T,>() => {
    let resolve!: (value: T | PromiseLike<T>) => void;
    let reject!: (reason?: unknown) => void;
    const promise = new Promise<T>((pass, fail) => {
      resolve = pass;
      reject = fail;
    });
    return { promise, reject, resolve };
  };
  const queue = new StorageDraftSaveQueue();
  let active = 0;
  let maximumActive = 0;
  let resetCount = 0;
  const form = { dirty: true, name: "Final name", secret: "shown-once" };
  const runSave = async (gate: ReturnType<typeof deferred<void>>) => {
    active += 1;
    maximumActive = Math.max(maximumActive, active);
    try {
      await gate.promise;
    } finally {
      active -= 1;
    }
  };
  const reset = () => {
    resetCount += 1;
    form.dirty = false;
    form.name = "";
    form.secret = "";
  };

  const autosaveGate = deferred<void>();
  const finalSaveGate = deferred<void>();
  const autosave = queue.enqueue(() => runSave(autosaveGate));
  const failedNavigation = queue.beforeNavigation(
    () => runSave(finalSaveGate),
    reset,
  );
  autosaveGate.resolve();
  await autosave;
  finalSaveGate.reject(new Error("write failed"));
  await assert.rejects(failedNavigation, /write failed/);

  assert.equal(maximumActive, 1);
  assert.equal(resetCount, 0);
  assert.deepEqual(form, { dirty: true, name: "Final name", secret: "shown-once" });

  const retryGate = deferred<void>();
  const successfulNavigation = queue.beforeNavigation(
    () => runSave(retryGate),
    reset,
  );
  const resetSeenByNextSave = queue.enqueue(async () => resetCount);
  assert.equal(resetCount, 0);
  retryGate.resolve();
  await successfulNavigation;
  assert.equal(maximumActive, 1);
  assert.equal(resetCount, 1);
  assert.equal(await resetSeenByNextSave, 1);
});

test("storage drafts ride the sidebar rail like project drafts do", () => {
  const html = renderToStaticMarkup(
    <ProjectSidebar
      projects={[]}
      drafts={[]}
      activeDraftId={null}
      storages={[
        cloudSummary(),
        folderSummary({ storage_id: "storage-future", name: "Future storage", provider: "dropbox", provider_label: "dropbox", icon: "storage", available: false, summary: [] }),
      ]}
      storageDrafts={[cloudDraft({ name: "Half-typed cloud" })]}
      storageProviders={[cloudProvider, folderProvider]}
      storageUsage={{ "storage-cloud": 1 }}
      storageIssues={{}}
      activeProjectId={null}
      activeStorageId="storage-drafted"
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      catalogIndexOpen={false}
      storageIndexOpen={false}
      settingsOpen={false}
      loading={false}
      busy={false}
      activityOpen={false}
      unreadLogs={0}
      onOpenProjects={() => undefined}
      onSelectProject={() => undefined}
      onRemoveProject={() => undefined}
      onSelectDraft={() => undefined}
      onDiscardDraft={() => undefined}
      onToggleActivity={() => undefined}
      onAddProject={() => undefined}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogIndex={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
      onOpenStorageIndex={() => undefined}
      onOpenSettings={() => undefined}
    />,
  );
  const rail = html.slice(html.indexOf('aria-label="Storages"'));
  assert.match(rail, /aria-label="Open Cloud storage"[\s\S]*?data-icon="cloud"[\s\S]*?<span>Cloud storage<\/span>/);
  assert.match(rail, /aria-label="Open Future storage\. Storage issue: This build has no dropbox adapter"[\s\S]*?data-icon="drive"/);
  assert.match(rail, /sync-sidebar-storage-warning/);
  assert.match(rail, /sidebar-profile-item sync-sidebar-draft active"[\s\S]*?aria-label="Continue setting up Half-typed cloud — resumable storage draft"[\s\S]*?data-icon="cloud"[\s\S]*?<span>Half-typed cloud<\/span><span class="sync-draft-badge">Draft<\/span>/);
  assert.match(rail, /title="Discard the local draft for Half-typed cloud; external storage resources are kept"/);
  assert.doesNotMatch(rail, /prepared-access-key|api_secret/);
});

/* Declared derivations and help links: the form fills a field from another
   as it is typed and opens the descriptor's documentation, without knowing
   any provider's URL shape itself. */
const derivingProvider: StorageProviderDescriptor = {
  ...cloudProvider,
  key: "cloud-d",
  fields: [
    { field_id: "api_url", label: "API URL", kind: "url", required: true, help: "How to find the API URL", help_url: "https://docs.example.test/api-url" },
    {
      field_id: "bucket",
      label: "Bucket name",
      kind: "text",
      required: true,
      help: "How to create a bucket",
      help_url: "https://docs.example.test/bucket",
      derive: { source: "api_url", pattern: "^https://([a-z0-9]+)\\.example\\.test/([a-z0-9-]+)$", group: 2 },
    },
    {
      field_id: "account_id",
      label: "Account ID",
      kind: "text",
      required: true,
      derive: { source: "api_url", pattern: "^https://([a-z0-9]+)\\.example\\.test/([a-z0-9-]+)$", group: 1 },
    },
    { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true },
  ],
  secrets: [
    { secret_id: "api_secret", label: "API Secret", required: true, placeholder: "Example: a1B2…x9Y0", help: "How to create a secret", help_url: "https://docs.example.test/secret" },
  ],
};

test("typing the source field fills every derived field and an unmatched value clears them", () => {
  const filled = applyFieldDerivations(
    derivingProvider,
    { api_url: " https://acct42.example.test/team-backup ", bucket: "", account_id: "", access_key_id: "" },
    "api_url",
  );
  assert.equal(filled.bucket, "team-backup");
  assert.equal(filled.account_id, "acct42");
  assert.equal(filled.access_key_id, "", "fields without a rule are untouched");
  const cleared = applyFieldDerivations(derivingProvider, { ...filled, api_url: "https://broken" }, "api_url");
  assert.equal(cleared.bucket, "");
  assert.equal(cleared.account_id, "");
  const untouched = applyFieldDerivations(derivingProvider, filled, "bucket");
  assert.deepEqual(untouched, filled, "editing a derived field never rewrites its source");
  assert.equal(fieldsSet(derivingProvider, { name: "Team", settings: filled, typedSecrets: {} }, {}).total, 5);
});

test("help marks are buttons that carry the descriptor's documentation link and placeholders come from the descriptor", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      descriptor={derivingProvider}
      form={{ name: "Team", settings: { api_url: "", bucket: "", account_id: "", access_key_id: "" }, typedSecrets: {} }}
      secretPresence={{}}
      disabled={false}
      onChange={() => {}}
    />,
  );
  assert.match(html, /<button type="button" class="field-help" title="How to create a bucket"/);
  assert.match(html, /title="How to create a secret"/);
  assert.match(html, /placeholder="Example: a1B2…x9Y0"/);
  assert.equal((html.match(/class="field-help"/g) ?? []).length, 3, "every declared help mark renders, and only those");
  assert.doesNotMatch(html, /docs\.example\.test/, "the link is opened by the mark, never rendered as navigation");
});


/* A suggested name follows the declared source field while the name is still
   Mallard's own, and never overwrites one the user typed. */
const namingFolderProvider: StorageProviderDescriptor = {
  ...folderProvider,
  key: "local-n",
  default_name: "Local storage",
  name_suggestion: { source: "directory", pattern: "([^/\\\\]+)[/\\\\]*$", group: 1 },
};

test("a new draft's numbered default name follows the chosen folder until the user types a name", () => {
  const fresh = { name: "Local storage 2", settings: { directory: "" }, typedSecrets: {} };
  assert.equal(storageNameIsAuto(namingFolderProvider, fresh), true);
  assert.equal(suggestStorageName(namingFolderProvider, { directory: "/Volumes/backup/team-configs/" }), "team-configs");
  assert.equal(suggestStorageName(namingFolderProvider, { directory: "" }), null, "no folder, no suggestion");
  const suggested = { ...fresh, name: "team-configs", settings: { directory: "/Volumes/backup/team-configs" } };
  assert.equal(storageNameIsAuto(namingFolderProvider, suggested), true, "the previous suggestion may be replaced");
  const typed = { ...suggested, name: "Shared team backup" };
  assert.equal(storageNameIsAuto(namingFolderProvider, typed), false, "a typed name is the user's");
  assert.equal(storageNameIsAuto({ ...namingFolderProvider, name_suggestion: null }, fresh), true);
  assert.equal(suggestStorageName({ ...namingFolderProvider, name_suggestion: null }, { directory: "/x/y" }), null);
});

test("the editor fills the suggested name from the source field and leaves a typed name alone", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      descriptor={namingFolderProvider}
      form={{ name: "team-configs", settings: { directory: "/Volumes/backup/team-configs" }, typedSecrets: {} }}
      secretPresence={{}}
      disabled={false}
      onChange={() => {}}
    />,
  );
  assert.match(html, /value="team-configs"/, "the suggested name sits in the ordinary editable name input");
  assert.doesNotMatch(html, /readonly|readOnly/);
});

/* The exact fetch of a new draft brings the assistant's own secret back for
   review: it sits in the field like a typed value, Show reveals it, and it
   travels no further unless the user changes it. */
test("a revealed draft secret fills the field for review and is not re-sent until edited", () => {
  const revealedDraft = cloudDraft({
    settings: { api_url: "https://example.test/team", access_key_id: "key-id" },
    secrets: [{ secret_id: "api_secret", present: true, value: "minted-by-the-agent" }],
  });
  const form = formFromDraft(cloudProvider, revealedDraft);
  assert.equal(form.typedSecrets.api_secret, "minted-by-the-agent");
  assert.deepEqual(form.persistedSecrets, { api_secret: "minted-by-the-agent" });
  assert.deepEqual(secretOperationsFromForm(form, secretPresenceOf(revealedDraft)), {}, "unchanged, it is already stored");
  assert.deepEqual(
    secretOperationsFromForm({ ...form, typedSecrets: { api_secret: "replaced-by-hand" } }, secretPresenceOf(revealedDraft)),
    { api_secret: { op: "replace", value: "replaced-by-hand" } },
  );
  const html = renderToStaticMarkup(
    <StorageEditor
      descriptor={cloudProvider}
      form={form}
      secretPresence={secretPresenceOf(revealedDraft)}
      disabled={false}
      onChange={() => undefined}
    />,
  );
  assert.match(html, /type="password"[^>]*value="minted-by-the-agent"/, "masked in the field, Show reveals it");
  assert.match(html, /sync-secret-reveal[^>]*>Show</);
  // Listings and edit drafts carry presence only; the form treats them as before.
  const redacted = formFromDraft(cloudProvider, cloudDraft({ secrets: [{ secret_id: "api_secret", present: true }] }));
  assert.deepEqual(redacted.typedSecrets, {});
});

// Execute the production handler with isolated command and dialog boundaries.
// This tests revision capture across the actual asynchronous confirmation.
function clearHarness(approve: boolean, changeRevision = false, failOnce = false) {
  const source = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const start = source.indexOf("  const clearStorageData = async");
  const end = source.indexOf("  const removeStorage = async", start);
  const { transformSync } = createRequire(`${process.cwd()}/package.json`)("esbuild") as typeof import("esbuild");
  const handler = transformSync(source.slice(start, end), { loader: "ts" }).code;
  const configRef = { current: { revision: 7, storages: [{ storage_id: "storage-local", name: "Test R2" }] } };
  const calls: unknown[][] = [];
  const errors: unknown[] = [];
  const confirmations: { message: string; options: { okLabel: string } }[] = [];
  let failures = failOnce ? 1 : 0;
  let refreshes = 0;
  const noop = () => {};
  const context = {
    configRef,
    hostWorkflowEpoch: { current: 1 },
    confirm: async (message: string, options: { okLabel: string }) => {
      confirmations.push({ message, options });
      if (changeRevision) configRef.current.revision = 8;
      return approve;
    },
    projectSyncApi: { clearStorageData: async (...args: unknown[]) => {
      calls.push(args);
      if (failures-- > 0) throw new Error("Some data was deleted. Retry Clear storage data.");
      return { repository_objects_deleted: 2, library_objects_deleted: 1 };
    } },
    setError: (error: unknown) => errors.push(error),
    setBusy: noop, closePullReview: noop, pullOutcomeInputs: { current: null },
    setPendingPush: noop, setPushError: noop,
    refresh: async () => { refreshes++; },
    setStorageRefreshEpoch: noop, notify: noop,
    errorMessage: (reason: Error) => reason.message,
  };
  const run = new Function("context", `const { ${Object.keys(context).join(", ")} } = context; ${handler}; return clearStorageData("storage-local");`);
  return { run: () => run(context) as Promise<void>, calls, errors, confirmations, refreshes: () => refreshes };
}

test("storage clear cancellation sends no command", async () => {
  const flow = clearHarness(false);
  await flow.run();
  assert.deepEqual(flow.calls, []);
  assert.equal(flow.refreshes(), 0);
  assert.match(flow.confirmations[0].message, /Test R2/);
  assert.match(flow.confirmations[0].message, /every synced project repository and Library resource/);
  assert.match(flow.confirmations[0].message, /sync has stopped on all machines/);
  assert.equal(flow.confirmations[0].options.okLabel, "Sync stopped — clear");
});

test("storage clear sends the revision captured before confirmation", async () => {
  const flow = clearHarness(true, true);
  await flow.run();
  assert.deepEqual(flow.calls, [["storage-local", 7]]);
});

test("storage clear failure retains local review state and permits manual retry", async () => {
  const flow = clearHarness(true, false, true);
  await flow.run();
  assert.equal(flow.refreshes(), 0);
  assert.match(String(flow.errors.at(-1)), /Some data was deleted/);
  await flow.run();
  assert.equal(flow.calls.length, 2);
  assert.equal(flow.refreshes(), 1);
});

test("the existing clear menu supports both clear modes and hides unavailable storage", () => {
  const source = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const expression = source.match(/const supportsManagedClear = (.*);/)?.[1];
  assert.ok(expression);
  const visible = new Function("storageAvailable", "descriptor", `return ${expression};`);
  assert.equal(visible(true, cloudProvider), true);
  assert.equal(visible(true, folderProvider), true);
  assert.equal(visible(true, { ...cloudProvider, clear_mode: "unavailable" }), false);
  assert.equal(visible(false, cloudProvider), false);
  assert.equal(visible(true, undefined), false);
  assert.match(source, /supportsManagedClear \? \[\{\s*id: "clear",\s*label: "Clear storage data…"/);
});

const dropboxProvider: StorageProviderDescriptor = {
    key: "dropbox", config_version: 1, display_name: "Dropbox", description: "Shared namespace",
    display_rank: 30, icon: "cloud", clear_mode: "revision_checked", default_name: "Dropbox storage",
    fields: [{ field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true }],
    secrets: [
      { secret_id: "access_token", label: "Access token", required: true },
      { secret_id: "refresh_token", label: "Refresh token", required: true },
    ],
    managed_secret_id: "refresh_token", local_setup: "dropbox",
    setup_guide_url: "https://www.dropbox.com/developers/reference/auth-types",
    manual_setup_hint: "Use an existing shared or team namespace.",
};

test("Dropbox declares direct setup and keeps the manual connection fields", () => {
  const descriptor = dropboxProvider;
  const form = { name: "Dropbox storage 1", settings: { namespace_id: "shared-42" }, typedSecrets: {} };
  const fresh = renderToStaticMarkup(
    <StorageEditor descriptor={descriptor} form={form} secretPresence={{ access_token: true }}
      disabled={false} onChange={() => {}}
      setupSlot={<button type="button">Connect Dropbox</button>} />,
  );
  assert.match(fresh, /Namespace ID/);
  assert.match(fresh, /Access token/);
  assert.doesNotMatch(fresh, /Refresh token/);
  assert.match(fresh, /Setup guide/);
  assert.match(fresh, /Use an existing shared or team namespace/);
  assert.match(fresh, /Dropbox setup/);
  assert.match(fresh, /Connect Dropbox/);
  assert.doesNotMatch(fresh, /readOnly/);
  assert.equal(fieldsSet(descriptor, form, { access_token: true }).total, 2);
  assert.equal(storageDraftReady(descriptor, form, { access_token: true }), true);
  // Settled, the same storage is the locked form: the namespace read-only,
  // both tokens masked and revealable, nothing to type.
  const locked = renderToStaticMarkup(
    <StorageLockedForm
      storage={cloudSummary({ provider: "dropbox", provider_label: "Dropbox", settings: { namespace_id: "shared-42" },
        secrets: [{ secret_id: "access_token", present: true }, { secret_id: "refresh_token", present: true }] })}
      descriptor={descriptor}
      disabled={false}
      onRename={async () => true}
      onRevealSecret={async () => "token"}
      onTest={() => undefined}
      testing={false}
      verifiedAt={null}
      reconnect={{ open: false, onOpen: () => undefined, band: null }}
    />,
  );
  assert.match(locked, /readOnly=""[^>]*value="shared-42"/);
  assert.match(locked, /Access token[\s\S]*?aria-label="Copy Access token"[\s\S]*?Refresh token[\s\S]*?aria-label="Copy Refresh token"/);
  assert.equal((locked.match(/type="password"[^>]*readOnly=""/g) ?? []).length, 2);
  assert.equal((locked.match(/>Show</g) ?? []).length, 2);
  assert.match(locked, /Access renews automatically\./);
  assert.doesNotMatch(locked, /Connect Dropbox|Manual setup|Stored — type to replace/);
});

test("Dropbox OAuth credentials use the same review and replacement field as R2", () => {
  const draft = cloudDraft({ provider: "dropbox", provider_label: "Dropbox", name: "Dropbox storage 1",
    settings: { namespace_id: "shared-42" },
    secrets: [{ secret_id: "access_token", present: false }, { secret_id: "refresh_token", present: true, value: "refresh-review-canary" }],
  });
  const form = formFromDraft(dropboxProvider, draft);
  const presence = secretPresenceOf(draft);
  assert.deepEqual(form.typedSecrets, { refresh_token: "refresh-review-canary" });
  assert.deepEqual(secretOperationsFromForm(form, presence), {});
  assert.equal(storageDraftReady(dropboxProvider, form, presence), true);
  assert.deepEqual(fieldsSet(dropboxProvider, form, presence), { set: 2, total: 2 });
  assert.equal(storageDraftReady(dropboxProvider, { ...form, settings: {} }, presence), false);
  assert.equal(storageDraftReady(dropboxProvider, { ...form, typedSecrets: {} }, {}), false);
  {
    const html = renderToStaticMarkup(<StorageEditor descriptor={dropboxProvider} form={form} secretPresence={presence}
      disabled={false} onChange={() => {}}
      setupSlot={<button type="button">Connect Dropbox</button>} />);
    // Presence alone never says "Connected": that word waits for an
    // observed success on the transport.
    assert.match(html, /Access renews automatically\. Test the connection to verify it/);
    assert.doesNotMatch(html, /Connected\./);
    assert.match(html, /value="shared-42"/);
    assert.doesNotMatch(html, /readOnly/);
    assert.match(html, /Connect Dropbox/);
    assert.match(html, /Refresh token/);
    assert.doesNotMatch(html, /Access token/);
    const credentialInput = html.match(/<input type="password"[^>]*>/)?.[0] ?? "";
    assert.match(credentialInput, /placeholder="Stored — type to replace"/);
    assert.match(credentialInput, /value="refresh-review-canary"/);
    assert.doesNotMatch(credentialInput, /readOnly|required=""/);
    assert.match(html, /sync-secret-reveal[^>]*>Show<\/button>/);
    assert.doesNotMatch(html, />Stored<|Manual setup/);
  }
  assert.deepEqual(secretOperationsFromForm(form, presence), {}, "viewing a saved token does not replace it");
  assert.deepEqual(secretOperationsFromForm({ ...form, typedSecrets: { refresh_token: "replacement-canary" } }, presence), {
    refresh_token: { op: "replace", value: "replacement-canary" },
  });
});

test("the guide row and the reconnect band say what the transport last observed", () => {
  const draft = cloudDraft({ provider: "dropbox", provider_label: "Dropbox", name: "Dropbox storage 1",
    settings: { namespace_id: "shared-42" }, edits_existing: true,
    secrets: [{ secret_id: "access_token", present: false }, { secret_id: "refresh_token", present: true }],
  });
  const form = formFromDraft(dropboxProvider, draft);
  const presence = secretPresenceOf(draft);
  const storage = (connection: StorageConnectionHealth | null) => cloudSummary({
    provider: "dropbox", provider_label: "Dropbox", settings: { namespace_id: "shared-42" },
    secrets: [{ secret_id: "access_token", present: false }, { secret_id: "refresh_token", present: true }],
    connection,
  });
  const band = (connection: StorageConnectionHealth | null) => (
    <StorageReconnectBand descriptor={dropboxProvider} form={form} secretPresence={presence} disabled={false}
      connection={connection} onChange={() => {}} setupSlot={<button type="button">Reconnect Dropbox</button>}
      verbs={<button type="button">Save credential</button>} />
  );
  const render = (connection: StorageConnectionHealth | null, open = false) => renderToStaticMarkup(
    <StorageLockedForm storage={storage(connection)} descriptor={dropboxProvider} disabled={false}
      onRename={async () => true} onRevealSecret={async () => ""} onTest={() => undefined} testing={false} verifiedAt={null}
      reconnect={{ open, onOpen: () => undefined, band: band(connection) }} />,
  );
  const unknown = render(null);
  assert.match(unknown, /role="note"/);
  assert.match(unknown, /Configured\. Test the connection to verify it\. Access renews automatically\./);
  assert.doesNotMatch(unknown, /Connected\.|Reconnect…|sync-storage-reconnect/);

  const refusal: StorageConnectionHealth = {
    state: "reconnect_required", code: "dropbox_reconnect_required", observed_at: 2,
    message: "Dropbox rejected the saved authorization. Reconnect Dropbox to restore access.",
    evidence: { endpoint: "oauth2/token", http_status: 400, error_tag: "invalid_grant", request_id: "req-1" },
  };
  const refused = render(refusal);
  assert.match(refused, /role="alert"/);
  assert.match(refused, /data-icon="alert-triangle"/);
  assert.match(refused, /storage-setup-guide-error">Dropbox rejected the saved authorization\. Reconnect Dropbox to restore access\.</);
  // The one door: Reconnect… in the row; the refused token wears the chip;
  // Test waits until the credential is replaced.
  assert.match(refused, /storage-setup-agent-cta"[^>]*>[\s\S]*?Reconnect…/);
  assert.match(refused, /Refresh token[\s\S]*?sync-storage-refused-chip/);
  assert.match(refused, /<button type="button" class="btn" disabled="">Test connection<\/button>/);
  assert.doesNotMatch(refused, /Connected|req-1|sync-storage-reconnect/);

  // Open, the band sits under the row with the lane and the replaceable
  // token only — the namespace stays locked below it.
  const opened = render(refusal, true);
  assert.match(opened, /class="sync-storage-reconnect" role="region" aria-label="Reconnect Dropbox"/);
  assert.match(opened, /Dropbox sign-in[\s\S]*?Reconnect Dropbox/);
  assert.match(opened, /placeholder="Rejected by Dropbox — paste a replacement"/);
  assert.match(opened, /Same destination — only the credential changes\./);
  assert.match(opened, /Save credential/);
  assert.doesNotMatch(opened, /Reconnect…/);
  assert.ok(opened.indexOf("sync-storage-reconnect") < opened.indexOf('value="shared-42"'));
  assert.match(opened, /readOnly=""[^>]*value="shared-42"/);

  const expired = renderToStaticMarkup(band({ state: "token_expired", code: "dropbox_token_expired", message: "The Dropbox access token expired.", observed_at: 3 }));
  assert.match(expired, /placeholder="Expired — paste a replacement"/);
  // A temporary failure suspends nothing and changes no copy.
  const unreachable = render({ state: "unreachable", code: "dropbox_request_failed", message: "Dropbox could not be reached.", observed_at: 4 });
  assert.match(unreachable, /role="note"/);
  assert.doesNotMatch(unreachable, /could not be reached|Reconnect…/);
});

test("connection helpers name the provider and print only allowlisted facts", () => {
  const health = (state: StorageConnectionHealth["state"]): StorageConnectionHealth => (
    { state, code: "code", message: "Message.", observed_at: 1 }
  );
  assert.equal(connectionSuspends(undefined), false);
  assert.equal(connectionSuspends(health("connected")), false);
  assert.equal(connectionSuspends(health("unreachable")), false);
  for (const state of ["reconnect_required", "token_expired", "permission_denied"] as const) {
    assert.equal(connectionSuspends(health(state)), true);
  }
  assert.equal(storageConnectionIssue(cloudSummary()), null);
  assert.equal(storageConnectionIssue(cloudSummary({ connection: health("connected") })), null);
  assert.equal(storageConnectionIssue(cloudSummary({ connection: health("unreachable") })), null);
  assert.equal(storageConnectionIssue(cloudSummary({ connection: health("reconnect_required") })), "Cloud A needs reconnection");
  assert.equal(storageConnectionIssue(cloudSummary({ connection: health("token_expired") })), "Cloud A needs reconnection");
  assert.equal(storageConnectionIssue(cloudSummary({ connection: health("permission_denied") })), "Cloud A denied access");
  assert.equal(connectionDetail(health("reconnect_required")), "Message.");
  assert.equal(
    connectionDetail({ ...health("reconnect_required"), evidence: { endpoint: "oauth2/token", http_status: 400, error_tag: "invalid_grant", request_id: "req-1" } }),
    "Message. (oauth2/token, HTTP 400, invalid_grant, request req-1)",
  );
  assert.equal(
    connectionDetail({ ...health("unreachable"), evidence: { endpoint: "files/download" } }),
    "Message. (files/download, no response)",
  );
});

test("a refused credential badges the index and the rail and leads to the settings", () => {
  const refused = cloudSummary({
    storage_id: "storage-refused", name: "Refused storage",
    connection: { state: "reconnect_required", code: "dropbox_reconnect_required", message: "Rejected.", observed_at: 1 },
  });
  const index = renderToStaticMarkup(
    <StorageIndexPage
      storages={[refused, cloudSummary()]}
      links={[]}
      storageIssues={{ "storage-refused": "Listing failed", "storage-cloud": "Bucket unreachable" }}
      drafts={[]}
      storageProviders={[cloudProvider]}
      busy={false}
      appActions={[]}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onAddStorage={() => undefined}
    />,
  );
  // The credential comes before the listing failure it caused, and the
  // chip's one word says what to do.
  assert.match(index, /aria-label="Open Refused storage\. Storage issue: Cloud A needs reconnection"/);
  assert.match(index, /aria-label="Open Cloud storage\. Storage issue: Bucket unreachable"/);
  assert.match(index, /<span>Reconnect<\/span>/);
  assert.match(index, /<span>Issue<\/span>/);
  const rail = renderToStaticMarkup(
    <ProjectSidebar
      projects={[]} drafts={[]} activeDraftId={null}
      storages={[refused]} storageDrafts={[]} storageProviders={[cloudProvider]}
      storageUsage={{}} storageIssues={{ "storage-refused": "Listing failed" }}
      activeProjectId={null} activeStorageId={null}
      catalogScopes={[]} activeCatalogScopeId={null} addingCatalogScope={false}
      catalogIndexOpen={false} storageIndexOpen settingsOpen={false}
      loading={false} busy={false} activityOpen={false} unreadLogs={0}
      onOpenProjects={() => undefined} onSelectProject={() => undefined} onRemoveProject={() => undefined}
      onSelectDraft={() => undefined} onDiscardDraft={() => undefined} onToggleActivity={() => undefined}
      onAddProject={() => undefined} onOpenStorage={() => undefined} onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined} onRemoveStorage={() => undefined} onAddStorage={() => undefined}
      onOpenCatalogIndex={() => undefined} onOpenCatalogScope={() => undefined} onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined} onOpenStorageIndex={() => undefined} onOpenSettings={() => undefined}
    />,
  );
  assert.match(rail, /aria-label="Open Refused storage\. Storage issue: Cloud A needs reconnection"[\s\S]*?sync-sidebar-storage-warning/);
  // Static markup cannot click; the sources pin where each door leads and
  // what the agent verb and the failure notice offer.
  for (const path of ["src/components/project-sync/StorageIndexPage.tsx", "src/components/project-sync/ProjectSidebar.tsx"]) {
    assert.match(readFileSync(path, "utf8"), /onOpenStorage\(storage\.storage_id\)/);
  }
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /<DropboxSetupLane/);
  assert.match(workspace, /connectionSuspends\(storage\.connection\) \? \([\s\S]*?onClick=\{\(\) => onOpenStorage\(storage\.storage_id\)\}[\s\S]*?Reconnect \{storage\.provider_label\}/);
  assert.match(workspace, /key: "storage-connection",[\s\S]*?message: connectionDetail\(storageConnection\)/);
  assert.match(workspace, /<StorageReconnectBand[\s\S]*?connection=\{storageConnection\}/);
  assert.match(workspace, /const beginReconnect = \(\) => \{[\s\S]*?ensureStorageEditSession\(savedEditedStorage\)/);
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /listen<StorageConnectionChangedEvent>\(\s*"project-sync-storage-connection-changed"/);
});

test("Dropbox preparation adopts only a complete exact draft with managed credentials", () => {
  const start = cloudDraft({ provider: "dropbox", provider_label: "Dropbox", name: "Dropbox storage 1", settings: {}, secrets: [], revision: 1 });
  const marker = createStorageSetupMarker(start, start.name);
  const ready = { ...start, revision: 2, settings: { namespace_id: "shared-42" }, secrets: [{ secret_id: "refresh_token", present: true }] };
  assert.equal(reconcileStorageSetup(marker, dropboxProvider, [ready]).kind, "adopt");
  assert.equal(reconcileStorageSetup(marker, dropboxProvider, [{ ...ready, secrets: [] }]).kind, "preserve");
  assert.equal(reconcileStorageSetup(marker, dropboxProvider, [{ ...ready, draft_id: "another-draft" }]).kind, "preserve");
  assert.equal(reconcileStorageSetup(marker, dropboxProvider, [{ ...ready, revision: 1 }]).kind, "preserve");
});

/* A provider that can look in its account declares every sentence the
   Use-existing door and lane print; the page never names it. */
const discoveryProvider: StorageProviderDescriptor = {
  ...cloudProvider,
  discovery: {
    unit: "bucket",
    unit_plural: "buckets",
    place: "your Cloud A account",
    create_hint: "A new bucket in your Cloud A account, made by the agent or by hand.",
    existing_hint: "Use a shared R2 bucket with your team, or connect your own.",
    existing_help: { label: "Set up team access", url: "https://example.test/account-members" },
    find_label: "Find in my account",
    access_hint: "Uses your Cloud A sign-in. Read-only: lists the buckets and reads one marker file in each.",
    scanning_hint: "Looking through your Cloud A account…",
    prepare_hint: "Cloud A issues an access key for this bucket only.",
  },
};

function scanResult(patch: Partial<StorageDiscoveryResult> = {}): StorageDiscoveryResult {
  return {
    scan_id: "scan-1",
    scope: "He Qu · 023e105f…72d0c353 · Default and EU jurisdictions",
    status: "partial",
    issues: ["1 bucket could not be read (team-archive: access denied)."],
    candidates: [
      { candidate_id: "c-1", name: "mallard-otter", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Jurisdiction", value: "Default" }], cautions: [], connected_storage_id: "storage-cloud" },
      { candidate_id: "c-2", name: "mallard-brave", identity: "ns 1234567890", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Jurisdiction", value: "EU" }, { label: "Created", value: "Sep 2" }], cautions: ["Viewer — Push will be refused"] },
      { candidate_id: "c-3", name: "mallard-gently", marker: "unsupported", marker_detail: "layout 2", facts: [], cautions: [] },
      { candidate_id: "c-4", name: "assets-prod", marker: "missing", facts: [], cautions: [] },
      { candidate_id: "c-5", name: "www-static", marker: "missing", facts: [], cautions: [] },
      { candidate_id: "c-6", name: "team-archive", marker: "unreadable", marker_detail: "access denied", facts: [], cautions: [] },
    ],
    ...patch,
  };
}

function renderDiscoveryPanel(state: DiscoveryState, selectedId: string | null = null, manualEntry = false) {
  return renderToStaticMarkup(
    <StorageDiscoveryPanel
      descriptor={discoveryProvider}
      discovery={discoveryProvider.discovery!}
      state={state}
      selectedId={selectedId}
      storages={[cloudSummary()]}
      manualEntry={manualEntry}
      disabled={false}
      onScan={() => undefined}
      onCancelScan={() => undefined}
      onSelect={() => undefined}
      onOpenStorage={() => undefined}
      onToggleManual={() => undefined}
      onCreateInstead={() => undefined}
    />,
  );
}

test("a provider that can look in its account asks a new draft Create new or Use existing before any lane", () => {
  const render = (
    mode: StorageSetupMode | null,
    manualEntry = false,
    descriptor = discoveryProvider,
  ) => renderToStaticMarkup(
    <StorageEditor
      descriptor={descriptor}
      form={emptyForm}
      secretPresence={{}}
      disabled={false}
      onChange={() => undefined}
      setupMode={{ mode, onChoose: () => undefined }}
      discoverySlot={<div id="discovery-slot" />}
      manualEntry={manualEntry}
      setupSlot={<button type="button">Connect Dropbox</button>}
    />,
  );
  const asking = render(null);
  assert.match(asking, /class="storage-mode-doors" role="radiogroup"/);
  assert.match(asking, /<strong>Create new<\/strong>/);
  assert.match(asking, /A new bucket in your Cloud A account, made by the agent or by hand\./);
  assert.match(asking, /<strong>Use existing<\/strong>/);
  assert.match(asking, /Use a shared R2 bucket with your team, or connect your own\./);
  assert.match(asking, /<\/button><a class="btn-link storage-mode-help" href="https:\/\/example\.test\/account-members" target="_blank" rel="noopener noreferrer">Set up team access <svg/);
  assert.doesNotMatch(asking, /Set up with an agent|Manual setup|API URL|discovery-slot/);

  // Create new is today's card, with the doors kept as a switch above it.
  const creating = render("create");
  assert.match(creating, /class="storage-mode-door" role="radio" aria-checked="true"><span class="storage-mode-door-title">[\s\S]*?<strong>Create new<\/strong>/);
  assert.match(creating, /aria-checked="false"><span class="storage-mode-door-title">[\s\S]*?<strong>Use existing<\/strong>/);
  assert.doesNotMatch(creating, /Set up with an agent|Connect Dropbox/);
  assert.match(creating, /API URL/);
  assert.doesNotMatch(creating, /discovery-slot/);

  // Use existing seats the look where the lanes were; the fields unfold
  // only for a connection pasted by hand.
  const existing = render("existing");
  assert.match(existing, /discovery-slot/);
  assert.doesNotMatch(existing, /Set up with an agent|Manual setup|API URL/);
  assert.match(render("existing", true), /discovery-slot[\s\S]*API URL/);

  // A provider that cannot look is past the question.
  const plain = render(null, false, cloudProvider);
  assert.doesNotMatch(plain, /storage-mode-doors/);
  assert.doesNotMatch(plain, /Set up with an agent/);
  assert.match(plain, /API URL/);
});

test("the discovery panel groups what the scan found by marker and offers only Mallard storages", () => {
  const html = renderDiscoveryPanel({ kind: "result", result: scanResult() }, "c-2");
  assert.match(html, /He Qu · 023e105f…72d0c353 · Default and EU jurisdictions · 6 buckets checked, read-only\./);
  assert.match(html, /2 Mallard storages · 2 without Mallard data · 1 unsupported · 1 could not be read/);
  // A short list is not an empty one: the partial scan says what it skipped.
  assert.match(html, /class="notice warning storage-found-notice" role="alert"/);
  assert.match(html, /Scan incomplete\.<\/strong> 1 bucket could not be read \(team-archive: access denied\)\. A Mallard storage there would not be listed\./);
  for (const head of ["Mallard storages", "Needs a newer Mallard", "Other buckets — no Mallard data", "Could not be checked"]) {
    assert.match(html, new RegExp(`<span>${head}</span>`));
  }
  // Four rows are drawn (the ones without Mallard data stay folded); only
  // the readable storage nobody is connected to can be chosen.
  assert.equal(html.match(/type="radio"/g)?.length, 4);
  assert.equal(html.match(/type="radio"[^>]*disabled=""/g)?.length, 3);
  assert.match(html, /aria-label="Use mallard-brave"[^>]*checked=""/);
  assert.match(html, /class="storage-found-row selected"/);
  assert.match(html, /<code>ns 1234567890<\/code>/);
  assert.match(html, /Connected · Cloud storage/);
  assert.match(html, /class="btn btn-ghost"[^>]*>Open</);
  assert.match(html, /Viewer — Push will be refused/);
  assert.match(html, /<span>Jurisdiction<\/span><b>EU<\/b>/);
  assert.match(html, /Mallard storage · layout 1/);
  assert.match(html, /Needs a newer Mallard · layout 2/);
  assert.match(html, /Could not read the marker · access denied/);
  assert.doesNotMatch(html, /assets-prod|www-static/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /Enter by hand/);
  assert.match(renderDiscoveryPanel({ kind: "result", result: scanResult() }, "c-2", true), /Hide fields/);
});

test("the discovery panel states the wait, the empty scan, and a scan that could not start", () => {
  assert.match(renderDiscoveryPanel({ kind: "idle" }), /Find in my account/);
  const scanning = renderDiscoveryPanel({ kind: "scanning" });
  assert.match(scanning, /status-loader[\s\S]*Looking through your Cloud A account…/);
  assert.match(scanning, />Cancel</);
  assert.doesNotMatch(scanning, /Find in my account/);

  const empty = renderDiscoveryPanel({
    kind: "result",
    result: scanResult({ status: "complete", issues: [], candidates: scanResult().candidates.filter((candidate) => candidate.marker === "missing") }),
  });
  assert.match(empty, /No Mallard storage in your Cloud A account\./);
  assert.match(empty, /2 buckets checked; none holds Mallard data\. Nothing was changed\./);
  assert.match(empty, /Create a new bucket instead/);
  assert.doesNotMatch(empty, /Scan incomplete|type="radio"/);

  const failed = renderDiscoveryPanel({ kind: "error", message: "Wrangler is not signed in on this Mac." });
  assert.match(failed, /class="notice danger storage-found-notice" role="alert"/);
  assert.match(failed, /Could not look in your Cloud A account\.<\/strong> Wrangler is not signed in on this Mac\./);
  assert.match(failed, /Try again/);

  // The browser step keeps the list in view, inert, and says whose turn it is.
  const preparing = renderDiscoveryPanel({ kind: "preparing", candidate: scanResult().candidates[1], result: scanResult() });
  assert.match(preparing, /Approve access to mallard-brave in your browser\. Cloud A issues an access key for this bucket only\./);
  assert.doesNotMatch(preparing, /Enter by hand|Find in my account/);
  assert.equal(preparing.match(/type="radio"[^>]*disabled=""/g)?.length, 4);
});

test("Dropbox selection uses the saved scan and exact draft through direct commands", () => {
  const source = readFileSync("src/components/project-sync/DropboxSetup.tsx", "utf8");
  assert.match(source, /saved.draft_id, saved.revision, login.session_id/);
  assert.match(source, /scan!.scan_id/);
  assert.match(source, /candidate\?\.candidate_id/);
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.match(api, /"discover_dropbox_storage"/);
  assert.match(api, /"prepare_dropbox_storage"/);
  assert.doesNotMatch(api, /launch_storage_setup_assistant|storage_setup_assistant_instruction|prepare_storage_draft_from_discovery/);
});

const noop = () => undefined;
const wranglerProvider: StorageProviderDescriptor = { ...discoveryProvider, managed_secret_id: null, local_setup: "wrangler" };
const account = { id: "1".repeat(32), name: "Personal" };

/** The lane drawn from one state of the hand-off; the verbs are inert. */
function r2Flow(state: Partial<R2SetupState>): R2SetupFlow {
  return {
    state: { ...idleR2State(), ...state },
    isRunning: () => false,
    start: noop, changeLogin: noop, continueWith: noop, setJurisdiction: noop, setName: noop, suggestAnother: noop,
    createAndAuthorize: noop, useBucket: noop, cancel: async () => undefined, retry: noop, scanAgain: noop, dismiss: noop,
  };
}
function renderLane(mode: R2SetupMode, state: Partial<R2SetupState>, disabled = false, bucketName?: string) {
  return renderToStaticMarkup(
    <div className="storage-setup-route-row agent r2-lane">
      <R2SetupLane
        mode={mode} flow={r2Flow(state)} disabled={disabled} descriptor={wranglerProvider} storages={[cloudSummary()]}
        bucketName={bucketName} onOpenStorage={noop} onCreateInstead={noop} onReset={noop}
      />
    </div>,
  );
}
const r2Scan = (patch: Partial<StorageDiscoveryResult> = {}) => scanResult({ scope: "Default, EU, US and FedRAMP", ...patch });

test("local Wrangler setup offers collapsed manual entry and opens details at final review", () => {
  const lane = (mode: "create" | "reconnect") => (
    <R2SetupLane
      mode={mode} flow={r2Flow({})} disabled={false} descriptor={wranglerProvider}
      storages={[]} onOpenStorage={noop} onCreateInstead={noop} onReset={noop}
    />
  );
  const render = (setupDone = false) => renderToStaticMarkup(<StorageEditor
    descriptor={wranglerProvider} form={formFromDraft(wranglerProvider, cloudDraft())}
    secretPresence={{}} disabled={false} onChange={() => {}} setupDone={setupDone}
    setupSlot={lane("create")}
  />);
  const creating = render();
  assert.match(creating, /class="storage-setup-route-row agent r2-lane"[^>]*>[\s\S]*?Wrangler setup[\s\S]*?Set up with Wrangler/);
  assert.match(creating, /Creates a bucket and issues its key\./);
  assert.match(creating, /Manual setup[\s\S]*aria-expanded="false" aria-label="Enter by hand"[^>]*><svg[^>]*data-icon="chevron-down"/);
  assert.doesNotMatch(creating, /API URL/);
  assert.doesNotMatch(creating, /Set up with an agent|Connect Dropbox|Run with Claude|Listening for your agent|Reset setup/);
  assert.match(render(true), /Manual setup[\s\S]*API URL/);
  // Settled, the lane rides the reconnect band: a new key for the same
  // bucket, pasted or issued, and no destination field to type in.
  const existing = renderToStaticMarkup(<StorageReconnectBand
    descriptor={wranglerProvider} form={formFromDraft(wranglerProvider, cloudDraft())}
    secretPresence={{}} disabled={false} onChange={() => {}} setupSlot={lane("reconnect")}
    verbs={<button type="button">Save credential</button>}
  />);
  assert.match(existing, /Wrangler setup[\s\S]*?Reconnect with Wrangler/);
  assert.match(existing, /Issues a new key for this bucket\./);
  assert.doesNotMatch(existing, /Manual setup|Set up with Wrangler|API URL/);
  assert.match(existing, /type="password"/);
  assert.match(existing, /Save credential/);
});

test("the Use-existing Wrangler route offers manual expansion and preserves prepared drafts", () => {
  const render = (setupDone = false, secretPresence = {}) => renderToStaticMarkup(<StorageEditor
    descriptor={{ ...wranglerProvider, setup_guide_url: "https://example.test/r2-setup" }} form={emptyForm} secretPresence={secretPresence} disabled={false} onChange={() => {}}
    setupMode={{ mode: "existing", onChoose: noop }} manualEntry={false}
    setupDone={setupDone}
    setupSlot={<R2SetupLane
      mode="existing" flow={r2Flow({})} disabled={false} descriptor={wranglerProvider}
      storages={[]} onOpenStorage={noop} onCreateInstead={noop} onReset={noop}
    />}
  />);
  const initial = render();
  assert.match(initial, /Find in my account/);
  assert.match(initial, /Manual setup[\s\S]*Setup guide[\s\S]*aria-expanded="false" aria-label="Enter by hand"[^>]*><svg[^>]*data-icon="chevron-down"/);
  assert.doesNotMatch(initial, /API URL|Access Key ID|API Secret/);
  const html = render(true);
  assert.match(html, /r2-lane[\s\S]*?Lists your buckets read-only[\s\S]*?Find in my account[\s\S]*?Manual setup[\s\S]*?Setup guide <svg[\s\S]*?API URL/);
  assert.match(html, /Access Key ID/);
  assert.match(html, /API Secret/);
  assert.doesNotMatch(html, /Enter by hand|Hide fields|Set up with Wrangler/);
  assert.match(render(false, { api_secret: true }), /Manual setup[\s\S]*Setup guide[\s\S]*API URL/);
});

test("the lane shows every step from the first one on, with Reset at its tail", () => {
  const html = renderLane("create", { phase: "check", running: true });
  const titles = [...html.matchAll(/class="r2-step-title">([^<]+)</g)].map((match) => match[1]);
  assert.deepEqual(titles, ["Wrangler", "Sign in", "Account", "Bucket", "Authorize key", "Review"]);
  assert.equal(html.match(/data-status="doing"/g)?.length, 1);
  assert.equal(html.match(/data-status="todo"/g)?.length, 5);
  assert.match(html, /Checking the install…/);
  assert.match(html, /status-loader/);
  assert.match(html, /class="btn storage-setup-guide-button btn-ghost"[^>]*>Cancel</);
  assert.match(html, /title="Discards the entire local setup draft and all setup progress\. Any bucket already created in Cloudflare and any connected storage are kept\."[^>]*>[\s\S]*?Reset setup/);
  assert.match(html, /Uses the Wrangler session; asks for a browser sign-in if there is none/);
  assert.match(html, /Confirmed before anything is created or listed/);
  assert.match(html, /Object Read &amp; Write · this bucket only/);
  assert.match(html, /Test connection, then Finish setup/);
  const existing = renderLane("existing", { phase: "check", running: true });
  assert.match(existing, /class="r2-step-title">Buckets</);
  assert.doesNotMatch(existing, /class="r2-step-title">Bucket</);
  assert.doesNotMatch(renderLane("create", {}), /Reset setup|r2-steps/);
});

test("Wrangler dependency failures supply the install command and its agent message", () => {
  const missing = renderLane("create", { phase: "check", failed: "wrangler_not_found" });
  assert.match(missing, /data-status="failed"/);
  assert.ok(missing.includes(WRANGLER_INSTALL_COMMAND));
  assert.match(missing, /Not on PATH/);
  assert.match(missing, /Copy command/);
  assert.match(missing, /Copy agent message/);
  assert.match(missing, /Try again/);
  const runtime = renderLane("create", { phase: "check", failed: "wrangler_runtime_unavailable" });
  assert.match(runtime, /Could not start · check the Node\.js and Wrangler installation/);
  assert.match(runtime, /Try again/);
  assert.doesNotMatch(runtime, /Copy command/);
  assert.match(renderLane("create", { phase: "check", failed: "helper_unavailable" }), /The Mallard setup helper is missing · restart Mallard/);
  const found = renderLane("create", { phase: "login", running: true, wrangler: { version: "4.125.0", path: "/opt/homebrew/bin/wrangler" } });
  assert.match(found, /data-status="done"[\s\S]*?Wrangler[\s\S]*?4\.125\.0 · <code>\/opt\/homebrew\/bin\/wrangler<\/code>/);
  assert.match(WRANGLER_INSTALL_AGENT_MESSAGE, /npm --version/);
  assert.match(WRANGLER_INSTALL_AGENT_MESSAGE, /wrangler --version/);
  assert.match(WRANGLER_INSTALL_AGENT_MESSAGE, /then stop/);
  assert.doesNotMatch(WRANGLER_INSTALL_AGENT_MESSAGE, /draft-id|mallard-cli|secret|token|credential/);
  assert.equal(r2SetupErrorCode({ code: "wrangler_not_found", message: "private-canary" }), "wrangler_not_found");
  assert.equal(r2SetupErrorCode({ code: "private-canary" }), "setup_failed");
  assert.equal(r2SetupErrorCode({ code: "constructor" }), "setup_failed");
  assert.equal(r2SetupErrorCode("private-canary"), "setup_failed");
  assert.equal(r2SetupErrorCode({ code: "cancelled" }), "cancelled");
});

test("browser steps hand the link over, count down its budget, and never open a browser themselves", () => {
  const link = { stage: "login" as const, url: "https://example.test/private-link-canary" };
  const login = renderLane("create", { phase: "login", running: true, link }, true);
  assert.match(login, /Sign in to Cloudflare in a browser on this computer · <span class="r2-countdown">3:00<\/span>/);
  assert.match(login, /<button[^>]*class="btn storage-setup-guide-button btn-primary"[^>]*>[\s\S]*?Copy login link</);
  assert.match(login, /Open browser/);
  assert.match(login, /class="btn storage-setup-guide-button btn-ghost"[^>]*>Cancel</);
  const buttonFor = (html: string, label: string) => html.match(new RegExp(`<button[^>]*>(?:(?!<button)[\\s\\S])*?${label}</button>`))?.[0] ?? "";
  for (const label of ["Copy login link", "Open browser", "Cancel"]) {
    assert.ok(buttonFor(login, label) && !buttonFor(login, label).includes('disabled=""'), `${label} stays live`);
  }
  assert.ok(buttonFor(login, "Reset setup").includes('disabled=""'), "Reset waits for the run");
  assert.doesNotMatch(login, /private-link-canary/);
  const consent = renderLane("create", { phase: "authorize", running: true, link: { stage: "bucket", url: link.url } }, true);
  assert.match(consent, /Approve access in a browser on this computer · <span class="r2-countdown">2:00<\/span>/);
  assert.match(consent, /Copy authorization link/);
  assert.match(consent, /Open browser/);
  assert.doesNotMatch(consent, /3:00|Copy login link/);
  // The same verbs serve the existing-bucket path.
  const existingConsent = renderLane("existing", { phase: "authorize", running: true, account, scan: r2Scan(), chosen: r2Scan().candidates[1], link: { stage: "bucket", url: link.url } }, true);
  assert.match(existingConsent, /Copy authorization link[\s\S]*?Open browser[\s\S]*?Cancel/);
  const signingIn = renderLane("create", { phase: "login", running: true }, true);
  assert.match(signingIn, /Starting sign-in…/);
  assert.doesNotMatch(signingIn, /Open browser|Copy login link/);
  const stopping = renderLane("create", { phase: "login", running: true, link, cancelling: true }, true);
  assert.match(stopping, /<button[^>]*disabled=""[^>]*>Cancelling…</);
  const stopped = renderLane("create", { phase: "login", cancelled: true });
  assert.match(stopped, /data-status="stopped"/);
  assert.match(stopped, /Stopped/);
  assert.match(stopped, /Try again/);
  assert.doesNotMatch(stopped, /Open browser|Copy login link/);
  assert.match(renderLane("create", { phase: "login", failed: "sign_in_required" }), /Sign-in did not complete[\s\S]*?Try again/);
  // The link is opened only by the Open browser button, never on arrival.
  const source = readFileSync("src/components/project-sync/R2Setup.tsx", "utf8");
  assert.equal(source.match(/openExternalUrl\(/g)?.length, 1);
  assert.match(source, /Open browser<\/>, \(\) => void openExternalUrl\(url\)/);
});

test("the account is confirmed before anything is listed or made, with Change login beside Continue", () => {
  const one = renderLane("create", { phase: "account", accounts: [account], account });
  assert.match(one, /Personal · <code>11111111…11111111<\/code>/);
  assert.doesNotMatch(one, /<select/);
  assert.match(one, /<button[^>]*class="btn storage-setup-guide-button btn-primary"[^>]*>Continue</);
  assert.doesNotMatch(one, /<button[^>]*disabled=""[^>]*>Continue/);
  assert.equal(one.match(/Change login/g)?.length, 1, "one Change login, on the account line");
  const choice = renderLane("existing", { phase: "account", accounts: [account, { id: "2".repeat(32), name: "Team" }] });
  assert.match(choice, /2 accounts in this session/);
  assert.match(choice, /<select[^>]*aria-label="Cloudflare account"/);
  assert.match(choice, /Choose an account/);
  assert.match(choice, /Personal · 11111111…11111111/);
  assert.match(choice, /Team/);
  assert.match(choice, /<button[^>]*disabled=""[^>]*>Continue/);
  assert.equal(choice.match(/Change login/g)?.length, 1);
  assert.doesNotMatch(choice, /storage-found-row|Looking across/, "nothing is listed before Continue");
  const picked = renderLane("create", { phase: "bucket", accounts: [account], account, name: "mallard-fox", proposal: "mallard-fox" });
  assert.match(picked, /data-status="done"[\s\S]*?Account[\s\S]*?Personal · <code>11111111…11111111<\/code>/);
  assert.doesNotMatch(picked, /<select|>Continue</);
  assert.match(picked, /role="radiogroup" aria-label="Jurisdiction"/);
  for (const label of ["Default", "EU", "US", "FedRAMP"]) assert.match(picked, new RegExp(`role="radio" aria-checked="(?:true|false)"[^>]*>${label}<`));
  assert.match(picked, /aria-checked="true"[^>]*>Default</);
  assert.match(picked, /Create &amp; authorize/);
  assert.match(picked, /class="r2-name-input"/);
  assert.equal(picked.match(/Change login/g)?.length, 1, "past the confirmation it moves to the sign-in line");
  const unavailable = renderLane("create", { phase: "account", failed: "account_unavailable", accounts: [account] });
  assert.match(unavailable, /No longer available · check the login[\s\S]*?Try again[\s\S]*?Change login/);
});

test("the bucket line proposes a name in a field, takes a rename, and hands a taken one back", () => {
  const proposed = renderLane("create", { phase: "bucket", account, name: "mallard-fox", proposal: "mallard-fox" });
  assert.match(proposed, /<input[^>]*class="r2-name-input"/);
  assert.match(proposed, /aria-label="Bucket name"/);
  assert.match(proposed, /value="mallard-fox"/);
  assert.doesNotMatch(proposed, /aria-invalid="true"/);
  assert.match(proposed, /· proposed by Mallard/);
  assert.equal(proposed.match(/Suggest another/g)?.length, 1);
  assert.match(proposed, /<button[^>]*class="btn storage-setup-guide-button btn-primary"[^>]*>Create &amp; authorize/);
  assert.doesNotMatch(proposed, /<button[^>]*disabled=""[^>]*>Create &amp; authorize/);
  assert.doesNotMatch(proposed, /Try again|named by Mallard/);
  const renamed = renderLane("create", { phase: "bucket", account, name: "team-backups", proposal: "mallard-fox" });
  assert.match(renamed, /value="team-backups"/);
  assert.match(renamed, /· renamed/);
  const invalid = renderLane("create", { phase: "bucket", account, name: "Team_Backups", proposal: "mallard-fox" });
  assert.match(invalid, /aria-invalid="true"/);
  assert.match(invalid, /· lowercase letters, numbers or hyphens only/);
  assert.match(invalid, /<button[^>]*disabled=""[^>]*>Create &amp; authorize/);
  assert.match(renderLane("create", { phase: "bucket", account, name: "ab", proposal: "mallard-fox" }), /· 3–63 characters/);
  assert.match(renderLane("create", { phase: "bucket", account, name: "team-", proposal: "mallard-fox" }), /· starts and ends with a letter or number/);
  const taken = renderLane("create", { phase: "bucket", account, failed: "bucket_name_taken", name: "team-backups", proposal: "mallard-fox" });
  assert.match(taken, /data-status="failed"[\s\S]*?Bucket/);
  assert.match(taken, /value="team-backups"/);
  assert.match(taken, /· already exists in this account · choose another name/);
  assert.match(taken, /role="radiogroup" aria-label="Jurisdiction"/, "nothing was created: the jurisdiction stays open");
  assert.match(taken, /<button[^>]*class="btn storage-setup-guide-button btn-primary"[^>]*>Create &amp; authorize/);
  assert.doesNotMatch(taken, /Try again/);
  const locked = renderLane("create", { phase: "bucket", account, name: "mallard-fox", proposal: "mallard-fox" }, true);
  assert.match(locked, /<input[^>]*class="r2-name-input"[^>]*disabled=""/);
  assert.match(locked, /<button[^>]*disabled=""[^>]*>Create &amp; authorize/);
  const reconnect = renderLane("reconnect", { phase: "bucket", account }, false, "team-backup");
  assert.doesNotMatch(reconnect, /r2-name-input|Suggest another|aria-label="Jurisdiction"/);
  const failedProposal = renderLane("create", { phase: "bucket", account, failed: "setup_failed" });
  assert.match(failedProposal, /Could not finish[\s\S]*?Try again/);
  assert.doesNotMatch(failedProposal, /r2-name-input/);
  for (const [name, issue] of [
    ["mallard-fox", null], ["a-1", null], ["a1", "3–63 characters"], ["x".repeat(64), "3–63 characters"],
    ["Team", "lowercase letters, numbers or hyphens only"], ["team backups", "lowercase letters, numbers or hyphens only"],
    ["-team", "starts and ends with a letter or number"], ["team-", "starts and ends with a letter or number"],
  ] as const) {
    assert.equal(bucketNameIssue(name), issue, name);
  }
  // The create asks for the bucket already made, else the field's name.
  const lane = readFileSync("src/components/project-sync/R2Setup.tsx", "utf8");
  assert.match(lane, /const requested = bucket \?\? name\.trim\(\);/);
  assert.match(lane, /api\.setupR2Storage\(saved\.draft_id, saved\.revision, account\.id, jurisdiction, requested, progressFor\(live\)\)/);
});

test("R2 activation failures explain dashboard setup and keep retry available", () => {
  const failed = r2SetupErrorCode({ code: "r2_not_enabled", message: "private-canary" });
  assert.equal(failed, "r2_not_enabled");
  for (const [mode, phase] of [["create", "create"], ["existing", "find"]] as const) {
    const html = renderLane(mode, { phase, failed, account, name: "mallard-fox" });
    assert.match(html, /R2 is not enabled for this account\. Complete R2 setup in the Cloudflare dashboard, then try again\./);
    assert.match(html, /Try again/);
    assert.doesNotMatch(html, /check the account permissions|private-canary/);
  }
  assert.match(renderLane("create", { phase: "create", failed: "permission_denied", account }), /Cloudflare refused access · check the account permissions/);
});

test("R2 setup gives specific recovery steps for Cloudflare failures", () => {
  for (const [code, message] of [
    ["authentication_failed", /Cloudflare could not verify the sign-in/],
    ["rate_limited", /Cloudflare received too many requests\. Wait, then try again\./],
    ["service_unavailable", /Could not reach Cloudflare or its service is temporarily unavailable/],
    ["request_rejected", /Cloudflare rejected the request\. Check the account, bucket name, and jurisdiction\./],
  ] as const) {
    assert.equal(r2SetupErrorCode({ code }), code);
    for (const [mode, phase] of [["create", "create"], ["existing", "find"]] as const) {
      const html = renderLane(mode, { phase, failed: code, account });
      assert.match(html, message);
      assert.match(html, /Try again/);
      if (code === "authentication_failed") assert.match(html, /<button[^>]*>[^]*?Change login<\/button>/);
      assert.doesNotMatch(html, /Creation result unknown/);
    }
  }
});

test("the bucket line keeps the created name through an expired consent and a stop", () => {
  const creating = renderLane("create", { phase: "create", running: true, account, name: "mallard-fox", proposal: "mallard-fox" });
  assert.match(creating, /Creating <code>mallard-fox<\/code>…/);
  assert.doesNotMatch(creating, /r2-name-input/);
  assert.match(creating, /<button[^>]*>Cancel</);
  const created = renderLane("create", { phase: "create", running: true, account, bucket: "mallard-fox", jurisdiction: "eu" });
  assert.match(created, /<code>mallard-fox<\/code> · EU · created/);
  const expired = renderLane("create", { phase: "authorize", failed: "authorization_timeout", account, bucket: "mallard-fox" });
  assert.match(expired, /Expired after 2 minutes · mallard-fox kept/);
  assert.match(expired, /Try again/);
  assert.match(expired, /<code>mallard-fox<\/code> · Default · created/);
  const stopped = renderLane("create", { phase: "authorize", cancelled: true, account, bucket: "mallard-fox" });
  assert.match(stopped, /Stopped · mallard-fox kept[\s\S]*?Try again/);
  const unknown = renderLane("create", { phase: "create", failed: "creation_unknown", account });
  assert.match(unknown, /Creation result unknown/);
  assert.doesNotMatch(unknown, /Try again/);
  assert.match(unknown, /Reset setup/);
});

test("the Buckets step lists the scan with the verb on the row and one Scan again", () => {
  const html = renderLane("existing", { phase: "pick", account, scan: r2Scan() });
  assert.match(html, /6 checked · read-only/);
  assert.equal(html.match(/Scan again/g)?.length, 1);
  assert.doesNotMatch(html, /type="radio"/);
  assert.match(html, /class="storage-found-row with-verb"/);
  assert.match(html, /<button[^>]*class="btn btn-primary storage-setup-guide-button"[^>]*aria-label="Use mallard-brave and authorize"[^>]*>Use &amp; authorize</);
  assert.equal(html.match(/Use &amp; authorize/g)?.length, 1, "only the storage nobody is connected to carries the verb");
  assert.match(html, /Connected · Cloud storage/);
  assert.match(html, /class="btn btn-ghost"[^>]*>Open</);
  const notice = html.match(/<div class="notice warning storage-found-notice"[\s\S]*?<\/p><\/div><\/div>/)?.[0] ?? "";
  assert.match(notice, /Scan incomplete\.<\/strong> 1 bucket could not be read \(team-archive: access denied\)\. A Mallard storage there would not be listed\./);
  assert.doesNotMatch(notice, /<button/, "the notice states; the Buckets line holds the verb");
  for (const head of ["Mallard storages", "Needs a newer Mallard", "Other buckets — no Mallard data", "Could not be checked"]) {
    assert.match(html, new RegExp(`<span>${head}</span>`));
  }
  assert.match(html, /<span>Jurisdiction<\/span><b>EU<\/b>/);
  assert.match(html, /Could not read the marker · access denied/);
  const finding = renderLane("existing", { phase: "find", running: true, account });
  assert.match(finding, /Looking across Default, EU, US and FedRAMP · read-only/);
  assert.match(finding, /<button[^>]*>Cancel</);
  assert.doesNotMatch(finding, /Scan again/);
  const empty = renderLane("existing", {
    phase: "pick", account,
    scan: r2Scan({ status: "complete", issues: [], candidates: r2Scan().candidates.filter((candidate) => candidate.marker === "missing") }),
  });
  assert.match(empty, /No Mallard storage in your Cloudflare account\./);
  assert.match(empty, /2 buckets checked across Default, EU, US and FedRAMP\. Nothing was changed\./);
  assert.match(empty, /Create a new bucket instead/);
  const expiredScan = renderLane("existing", { phase: "find", failed: "draft_changed", account });
  assert.match(expiredScan, /Scan expired[\s\S]*?Scan again/);
  assert.doesNotMatch(expiredScan, /Try again|storage-found-row/);
  const verifying = renderLane("existing", { phase: "verify", running: true, account, scan: r2Scan(), chosen: r2Scan().candidates[1] });
  assert.match(verifying, /<code>mallard-brave<\/code> · EU/);
  assert.match(verifying, /Checking <code>mallard-brave<\/code> is unchanged…/);
  assert.doesNotMatch(verifying, /storage-found-row/, "the list folds into the line once a bucket is taken");
});

test("the ready line states the facts in chips and keeps Reset", () => {
  const existing = renderLane("existing", { phase: "review", account, scan: r2Scan(), chosen: r2Scan().candidates[1] });
  assert.match(existing, /Ready for review\./);
  assert.match(existing, /<span>Bucket<\/span><b>mallard-brave<\/b>/);
  assert.match(existing, /<span>Jurisdiction<\/span><b>EU<\/b>/);
  assert.match(existing, /<span>Account<\/span><b>Personal<\/b>/);
  assert.match(existing, /<span>Key<\/span><b>this bucket only<\/b>/);
  assert.match(existing, /Reset setup/);
  assert.doesNotMatch(existing, /r2-steps|Scan again/);
  const created = renderLane("create", { phase: "review", account, bucket: "mallard-fox", jurisdiction: "us" });
  assert.match(created, /<span>Bucket<\/span><b>mallard-fox<\/b>/);
  assert.match(created, /<span>Jurisdiction<\/span><b>US<\/b>/);
});

test("the reconnect lane names the storage's bucket and authorizes without creating", () => {
  const html = renderLane("reconnect", { phase: "bucket", account }, false, "team-backup");
  assert.match(html, /<code>team-backup<\/code> · this storage’s bucket/);
  assert.match(html, /<button[^>]*class="btn storage-setup-guide-button btn-primary"[^>]*>Authorize</);
  assert.doesNotMatch(html, /aria-label="Jurisdiction"|Create &amp; authorize/);
  assert.match(html, /Test connection, then Save storage/);
  assert.match(renderLane("reconnect", { phase: "create", running: true, account, bucket: "team-backup" }, true, "team-backup"), /<code>team-backup<\/code> · kept/);
});

test("Wrangler completion loads the exact prepared draft without a watcher event", async () => {
  const saved = cloudDraft({ revision: 4 });
  const prepared = cloudDraft({ revision: 5 });
  const calls: unknown[] = [];
  const result = await loadPreparedR2Draft(saved, cloudProvider, () => true, {
    getStorageSetupDraft: async (id) => { calls.push(id); return prepared; },
  });
  assert.equal(result, prepared);
  assert.deepEqual(calls, [saved.draft_id]);
});

test("Wrangler completion cannot adopt a result after a form edit or navigation", async () => {
  const saved = cloudDraft({ revision: 4 });
  const prepared = cloudDraft({ revision: 5 });
  for (const staleAt of ["before", "load"]) {
    let current = staleAt !== "before";
    let loads = 0;
    const result = await loadPreparedR2Draft(saved, cloudProvider, () => current, {
      getStorageSetupDraft: async () => { loads += 1; current = false; return prepared; },
    });
    assert.equal(result, null);
    assert.equal(loads, staleAt === "load" ? 1 : 0);
  }
});

test("Wrangler completion rejects a changed identity", async () => {
  const saved = cloudDraft({ revision: 4 });
  for (const prepared of [null, saved, cloudDraft({ revision: 5, provider: "another-provider" }), cloudDraft({ revision: 5, draft_id: "another-draft" })]) {
    await assert.rejects(loadPreparedR2Draft(saved, cloudProvider, () => true, {
      getStorageSetupDraft: async () => prepared,
    }), { code: "draft_changed" });
  }
});

test("R2 setup uses a synchronous busy guard and blocks conflicting actions", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /const run = async[^]*?if \(r2\.isRunning\(\) \|\| dropbox\.isRunning\(\) \|\| managedCloud\.isRunning\(\)\) return;/);
  assert.match(workspace, /const saveStorageDraftBeforeNavigation = async[^]*?if \(r2\.isRunning\(\) \|\| dropbox\.isRunning\(\) \|\| managedCloud\.isRunning\(\)\) return false;/);
  assert.match(workspace, /const formLocked = busy \|\| !!runningAction \|\| r2\.state\.running \|\| dropbox\.state\.running;/);
  assert.match(workspace, /blocked: \(\) => busy \|\| !!runningAction,/);
  // Every run pins the draft it started on and saves it first.
  assert.match(workspace, /persist: \(\) => persistStorageSetupDraft\(session, editVersion\),/);
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.match(api, /invoke<void>\("setup_r2_storage", \{ draftId, expectedRevision, accountId, jurisdiction, bucket, onProgress: r2ProgressChannel\(onProgress\) \}\)/);
  assert.match(api, /invoke<string>\("propose_r2_bucket_name"\)/);
});

test("R2 suggested names keep the prefix and preserve custom names", () => {
  const descriptor: StorageProviderDescriptor = {
    ...cloudProvider, local_setup: "wrangler", default_name: "R2 storage",
    name_suggestion: { source: "bucket", pattern: "^\\s*(\\S.*?)\\s*$", group: 1 },
  };
  for (const name of ["", "R2 storage", "R2 storage 1"]) {
    assert.equal(storageNameIsAuto(descriptor, { name, settings: {} }), true);
  }
  assert.equal(suggestStorageName(descriptor, { bucket: "mallard-otter" }), "R2 mallard-otter");
  assert.equal(suggestStorageName(descriptor, { bucket: "team-backup" }), "R2 team-backup");
  assert.equal(suggestStorageName(descriptor, { bucket: "" }), null);
  assert.equal(storageNameIsAuto(descriptor, { name: "R2 mallard-otter", settings: { bucket: "mallard-otter" } }), true);
  for (const name of ["Team backup", "R2 team archive"]) {
    assert.equal(storageNameIsAuto(descriptor, { name, settings: { bucket: "mallard-otter" } }), false);
  }
  const automatic = { name: "R2 mallard-otter", settings: { bucket: "mallard-otter" } };
  const renamed = storageNameForSettings(descriptor, automatic, { bucket: "team-backup" });
  assert.equal(renamed, "R2 team-backup", "changing the bucket replaces the automatic name without another prefix");
  const cleared = { name: storageNameForSettings(descriptor, automatic, { bucket: "" }), settings: { bucket: "" } };
  assert.equal(cleared.name, "R2 storage");
  assert.equal(storageNameForSettings(descriptor, cleared, { bucket: "team-backup" }), "R2 team-backup");
  assert.equal(storageNameForSettings(descriptor, { ...automatic, name: "Team archive" }, { bucket: "team-backup" }), "Team archive");
});

// Dropbox uses the same setup rows, with direct browser authorization.
import { DropboxSetupLane, dropboxFolderNameIssue, type useDropboxSetup } from "../../src/components/project-sync/DropboxSetup";
type DropboxFlow = ReturnType<typeof useDropboxSetup>;
function dropboxFlow(state: Partial<DropboxFlow["state"]>): DropboxFlow {
  return {
    state: { phase: "idle", running: false, cancelling: false, error: null, link: null, login: null, account: null, name: "", proposal: null, creationStarted: false, scan: null, chosen: null, ...state },
    isRunning: () => false, start: noop, confirmAccount: noop, scan: noop, prepare: noop,
    cancel: async () => undefined, dismiss: noop, suggest: noop, setName: noop,
  };
}
function renderDropbox(state: Partial<DropboxFlow["state"]>, mode: "create" | "existing" | "reconnect" = "create") {
  return renderToStaticMarkup(<DropboxSetupLane flow={dropboxFlow(state)} mode={mode} descriptor={dropboxProvider} storages={[]}
    disabled={state.running ?? false} onOpenStorage={noop} onCreateInstead={noop} onNewSetup={noop} onReset={noop} />);
}
const dropboxLogin = { session_id: "opaque-session", account: { id: "dbid:test", name: "Test User", email: "test@example.test" }, folder_name: null, creation_started: false };

test("Dropbox consent offers the link to copy or open with Cancel on the line, never printed", () => {
  const html = renderDropbox({ phase: "login", running: true, link: "https://www.dropbox.com/oauth2/authorize?state=fixture" });
  assert.doesNotMatch(html, /oauth2\/authorize\?state=fixture/);
  for (const label of ["Copy authorization link", "Open browser", "Cancel"]) {
    const [, attributes] = html.match(new RegExp(`<button([^>]*)>(?:<svg[^>]*>(?:(?!</svg>)[^])*</svg> )?${label}</button>`)) ?? [];
    assert.ok(attributes !== undefined, `${label} button`);
    assert.doesNotMatch(attributes, /disabled/);
  }
  assert.match(html, /Approve access in a browser on this computer · <span class="r2-countdown">3:00<\/span>/);
  assert.match(html, /Reset setup/);
  assert.doesNotMatch(html, /Run with|Access code|app secret|Switch account/);
});
test("Dropbox before start says what the lane does and that consent is account-wide", () => {
  const html = renderDropbox({ phase: "idle" });
  assert.match(html, /storage-setup-agent-cta[^>]*>(?:<svg[^]*?<\/svg>)? Connect Dropbox</);
  assert.match(html, /covers your whole account, not only this folder/);
  assert.match(renderDropbox({ phase: "idle" }, "existing"), /Find in my Dropbox/);
  assert.match(renderDropbox({ phase: "idle" }, "reconnect"), /Reconnect Dropbox/);
});
test("Dropbox requires account confirmation, with Change login beside Continue", () => {
  const html = renderDropbox({ phase: "account", login: dropboxLogin, account: dropboxLogin.account });
  assert.match(html, /Test User · <code>test@example\.test<\/code>/);
  assert.match(html, /btn-primary[^>]*>Continue</);
  assert.match(html, /Change login/);
  assert.doesNotMatch(html, /Create &amp; connect<|Use &amp; connect<|Switch account/);
});
test("Dropbox names are editable and a conflict never silently chooses a replacement", () => {
  const html = renderDropbox({ phase: "prepare", error: "dropbox_folder_name_taken", name: "My backup", proposal: "Mallard fox", login: dropboxLogin, account: dropboxLogin.account, creationStarted: true });
  assert.match(html, /class="r2-name-input"[^>]*value="My backup"/);
  assert.match(html, /aria-label="Dropbox folder name"/);
  assert.match(html, /Suggest another/);
  assert.match(html, /btn-primary[^>]*>Create &amp; connect</);
  assert.match(html, /already exists in your Dropbox/);
  assert.match(html, /New setup for another account/);
  assert.doesNotMatch(html, /storage-setup-guide-error/);
  const proposed = renderDropbox({ phase: "folder", name: "Mallard fox", proposal: "Mallard fox", login: dropboxLogin, account: dropboxLogin.account });
  assert.match(proposed, /· proposed by Mallard/);
  const invalid = renderDropbox({ phase: "folder", name: "a/b", proposal: "Mallard fox", login: dropboxLogin, account: dropboxLogin.account });
  assert.match(invalid, /aria-invalid="true"/);
  assert.match(invalid, /<button[^>]*disabled=""[^>]*>Create &amp; connect</);
});
test("Dropbox folder validation rejects paths and bounds UTF-8 bytes", () => {
  for (const name of ["", ".", "..", " a", "a ", "a/b", "a\\b", "a\u0085", "é".repeat(128)]) assert.ok(dropboxFolderNameIssue(name));
  for (const name of ["My backup", "é".repeat(127)]) assert.equal(dropboxFolderNameIssue(name), null);
});
test("Dropbox discovery only offers valid unconnected Mallard folders, with Use & connect on the row", () => {
  const scan = scanResult();
  const html = renderDropbox({ phase: "pick", login: dropboxLogin, account: dropboxLogin.account, scan }, "existing");
  assert.equal(html.match(/>Use &amp; connect<\/button>/g)?.length, 1);
  assert.match(html, /Scan again/);
  assert.match(html, /Change login/);
  assert.match(html, /checked · read-only/);
});
test("Dropbox reconnect keeps the storage’s folder and ends at Save storage, the footer’s verb", () => {
  const prepare = renderDropbox({ phase: "prepare", running: true, name: "", login: dropboxLogin, account: dropboxLogin.account }, "reconnect");
  assert.match(prepare, /This storage’s folder · kept/);
  assert.match(prepare, /Renewing access…/);
  assert.match(prepare, /Test connection, then Save storage/);
  assert.doesNotMatch(prepare, /Create &amp; connect|Dropbox folder name|Finish setup/);
  const review = renderDropbox({ phase: "review", name: "", login: null, account: dropboxLogin.account }, "reconnect");
  assert.match(review, /Ready for review\./);
  assert.match(review, /Account<\/span><b>Test User<\/b>/);
  assert.match(review, /Access<\/span><b>account-wide · renews<\/b>/);
  assert.match(review, /Reset setup/);
  assert.doesNotMatch(review, /r2-steps|Finish setup/);
});
test("Dropbox timeout and revoked-token states say the reason on the Sign in line and permit fresh authorization", () => {
  for (const error of ["dropbox_authorization_timeout", "dropbox_reconnect_required"]) {
    const html = renderDropbox({ phase: "login", error });
    assert.match(html, /data-status="failed"[^]*?class="r2-step-fact" role="alert"/);
    assert.match(html, /btn-primary[^>]*>Try again</);
    assert.doesNotMatch(html, /Dropbox authorization link|Copy authorization link|Open browser|storage-setup-guide-error/);
  }
  const unknown = renderDropbox({ phase: "prepare", error: "dropbox_setup_outcome_unknown", name: "My backup", login: dropboxLogin, account: dropboxLogin.account, creationStarted: true });
  assert.match(unknown, /did not confirm the folder operation · inspect Dropbox before a new setup · the created folder is kept/);
  assert.doesNotMatch(unknown, /Try again/);
  assert.match(unknown, /New setup for another account/);
});

// Mallard Cloud signs in with GitHub and keeps the session as its only secret.
import { ManagedCloudSetupLane, type useManagedCloudSetup } from "../../src/components/project-sync/ManagedCloudSetup";
import { setupRouteIcon, setupRouteLabel } from "../../src/components/project-sync/StorageSettings";
const managedCloudProvider: StorageProviderDescriptor = {
    key: "managed_cloud", config_version: 1, display_name: "Mallard Cloud", description: "Mallard's own service",
    display_rank: 5, icon: "cloud", clear_mode: "unavailable", default_name: "Mallard Cloud",
    fields: [],
    secrets: [{ secret_id: "account_token", label: "Sign-in token", required: true }],
    managed_secret_id: "account_token", local_setup: "managed_cloud",
};
type ManagedCloudFlow = ReturnType<typeof useManagedCloudSetup>;
function managedCloudFlow(state: Partial<ManagedCloudFlow["state"]>): ManagedCloudFlow {
  return {
    state: { phase: "idle", running: false, cancelling: false, error: null, link: null, login: null, account: null, serviceOrigin: null, ...state },
    isRunning: () => false, start: noop, confirmAccount: noop, cancel: async () => undefined, dismiss: noop,
  };
}
function renderManagedCloud(state: Partial<ManagedCloudFlow["state"]>, mode: "create" | "reconnect" = "create", storages: StorageSummary[] = []) {
  return renderToStaticMarkup(<ManagedCloudSetupLane flow={managedCloudFlow(state)} mode={mode} storages={storages}
    disabled={state.running ?? false} onOpenStorage={noop} onReset={noop} />);
}
const managedCloudLogin = { session_id: "opaque-session", account: { id: "user_1", login: "octocat" }, service_origin: "https://mallard-ai.com" };

test("Mallard Cloud declares a sign-in lane, no manual fields, and one managed secret", () => {
  const form = { name: "Mallard Cloud", settings: {}, typedSecrets: {} };
  const fresh = renderToStaticMarkup(
    <StorageEditor descriptor={managedCloudProvider} form={form} secretPresence={{}}
      disabled={false} onChange={() => {}}
      setupSlot={<button type="button">Sign in with GitHub</button>} />,
  );
  assert.match(fresh, /Mallard Cloud setup/);
  assert.match(fresh, /Sign in with GitHub/);
  assert.doesNotMatch(fresh, /Manual setup|Enter by hand|Setup guide/);
  assert.deepEqual(fieldsSet(managedCloudProvider, form, {}), { set: 0, total: 1 });
  assert.equal(storageDraftReady(managedCloudProvider, form, {}), false);
  assert.equal(storageDraftReady(managedCloudProvider, form, { account_token: true }), true);
  assert.equal(setupRouteIcon("managed_cloud"), "cloud");
  assert.equal(setupRouteLabel("managed_cloud", "sign-in"), "Mallard Cloud sign-in");
  assert.equal(setupRouteLabel("dropbox", "setup"), "Dropbox setup");
  assert.equal(setupRouteLabel("wrangler", "sign-in"), "Wrangler setup");
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /<ManagedCloudSetupLane/);
  assert.match(workspace, /local_setup === "managed_cloud"/);
});
test("Mallard Cloud before start says what the lane does and who can read a push", () => {
  const html = renderManagedCloud({ phase: "idle" });
  assert.match(html, /storage-setup-agent-cta[^>]*>(?:<svg[^]*?<\/svg>)? Sign in with GitHub</);
  assert.match(html, /private to you and the collaborators you invite/);
  assert.match(html, /public only when you request a release/);
  assert.match(renderManagedCloud({ phase: "idle" }, "reconnect"), /Reconnect Mallard Cloud/);
});
test("Mallard Cloud sign-in offers the link to copy or open with Cancel on the line, never printed", () => {
  const html = renderManagedCloud({ phase: "login", running: true, link: "https://github.com/login/oauth/authorize?state=fixture" });
  assert.doesNotMatch(html, /oauth\/authorize\?state=fixture/);
  for (const label of ["Copy sign-in link", "Open browser", "Cancel"]) {
    const [, attributes] = html.match(new RegExp(`<button([^>]*)>(?:<svg[^>]*>(?:(?!</svg>)[^])*</svg> )?${label}</button>`)) ?? [];
    assert.ok(attributes !== undefined, `${label} button`);
    assert.doesNotMatch(attributes, /disabled/);
  }
  assert.match(html, /Sign in with GitHub in a browser on this computer · <span class="r2-countdown">3:00<\/span>/);
  assert.match(html, /Reset setup/);
});
test("Mallard Cloud confirms the account before saving, with Change login beside Continue", () => {
  const html = renderManagedCloud({ phase: "account", login: managedCloudLogin, account: managedCloudLogin.account, serviceOrigin: managedCloudLogin.service_origin });
  assert.match(html, /<code>octocat<\/code> on GitHub/);
  assert.match(html, /btn-primary[^>]*>Continue</);
  assert.match(html, /Change login/);
  assert.match(html, /A 90-day session · private to you and your collaborators/);
  const taken = renderManagedCloud(
    { phase: "account", error: "storage_managed_account_exists", login: managedCloudLogin, account: managedCloudLogin.account },
    "create",
    [cloudSummary({ storage_id: "storage-cloud", name: "Team cloud", provider: "managed_cloud", provider_label: "Mallard Cloud", settings: { account_id: "user_1", service_origin: "https://mallard-ai.com", account_login: "octocat" }, secrets: [] })],
  );
  assert.match(taken, /already connected · open its storage/);
  assert.match(taken, /btn-primary[^>]*>Open Team cloud</);
});
test("Mallard Cloud review names the account and the service, and reconnect ends at Save storage", () => {
  const review = renderManagedCloud({ phase: "review", account: managedCloudLogin.account, serviceOrigin: managedCloudLogin.service_origin });
  assert.match(review, /Ready for review\./);
  assert.match(review, /Account<\/span><b>octocat<\/b>/);
  assert.match(review, /Service<\/span><b>mallard-ai\.com<\/b>/);
  assert.doesNotMatch(review, /r2-steps/);
  const reconnect = renderManagedCloud({ phase: "access", running: true, account: managedCloudLogin.account, login: managedCloudLogin }, "reconnect");
  assert.match(reconnect, /this storage’s account/);
  assert.match(reconnect, /Saving the session…/);
  assert.match(reconnect, /Test connection, then Save storage/);
  const mismatch = renderManagedCloud({ phase: "account", error: "managed_cloud_account_mismatch", login: managedCloudLogin, account: managedCloudLogin.account }, "reconnect");
  assert.match(mismatch, /data-status="failed"[^]*?belongs to another GitHub account/);
});

// The website bridge: one chip in the path bar for the release state; Open on
// Mallard Cloud and Request public release (only when a request would be
// accepted) in the project crumb's menu.
import { releaseChip } from "../../src/components/project-sync/managedCloudLink";
test("Mallard Cloud release chips name each state and only a rejection reads as danger", () => {
  assert.deepEqual(releaseChip({ kind: "public" }).label, "Public");
  assert.deepEqual(releaseChip({ kind: "waiting_owner", requested_by: "user_2" }), {
    label: "Pending approval",
    severity: "plain",
    detail: "Public release awaits Owner approval.",
  });
  assert.deepEqual(releaseChip({ kind: "waiting_super_admin" }), {
    label: "Pending approval",
    severity: "plain",
    detail: "Public release awaits Admin approval.",
  });
  assert.equal(releaseChip({ kind: "snapshot_changed" }).severity, "warning");
  assert.equal(releaseChip({ kind: "rejected", reason: "Not yet" }).severity, "danger");
  assert.match(releaseChip({ kind: "rejected", reason: "Not yet" }).detail, /Not yet/);
  assert.equal(releaseChip({ kind: "private" }).severity, "plain");
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.doesNotMatch(workspace, /getManagedCloudLink/);
  assert.match(workspace, /activityHeader\.managedCloudStatus/);
  assert.match(workspace, /managedCloudRepositoryId \? workspaceActionPrefix : null/);
  assert.match(workspace, /label: "Open on Mallard Cloud"[^]*?openExternalUrl\(link\.website_url\)/);
  assert.match(workspace, /if \(link\.can_request_release\) \{[^]*?requestManagedCloudRelease\(/);
  assert.match(workspace, /\.\.\.managedCloudMenuItems,/);
  assert.doesNotMatch(workspace, /sync-path-cloud-controls/);
  assert.match(workspace, /window\.addEventListener\("focus", refresh\)/);
});

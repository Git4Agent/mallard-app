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
  findStorageProvider,
  formFromDraft,
  secretOperationsFromForm,
  secretPresenceOf,
  storageConnectionIssue,
  storageDraftReady,
  storageSubtitle,
  storageSubtitleIsPath,
  StorageEditor,
  type StorageEditorForm,
  applyFieldDerivations,
  suggestStorageName,
  storageNameIsAuto,
} from "../../src/components/project-sync/StorageSettings";
import {
  CatalogStorageRepositoryRow,
  createAgentSetupMarker,
  reconcileAgentSetup,
  StorageRepositoryDetails,
  StorageRepositoryRow,
} from "../../src/components/project-sync/ProjectLinksWorkspace";
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
  setup_assistant: {
    label: "Set up with an agent",
    description: "A coding agent runs the Mallard CLI and prepares this draft for your review.",
  },
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
  setup_assistant: null,
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

test("the editor renders exactly the fields, secrets, and assistant its descriptor declares", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      descriptor={cloudProvider}
      form={emptyForm}
      secretPresence={{}}
      disabled={false}
      editsExisting={false}
      onChange={() => undefined}
      agentSlot={<button type="button">Run with Claude Code</button>}
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
  // Two named routes from the descriptor's assistant: its label leads the
  // agent lane and seats the hand-off cluster; the manual lane introduces
  // the fields. No provider docs link, no provider name in the copy.
  assert.match(html, /storage-setup-route-row agent/);
  assert.match(html, /Set up with an agent/);
  assert.match(html, /A coding agent runs the Mallard CLI and prepares this draft for your review\./);
  assert.match(html, /Run with Claude Code/);
  assert.match(html, /Manual setup/);
  assert.match(html, /Enter an existing Cloud A connection in the fields below\./);
  assert.doesNotMatch(html, /Setup guide|Cloudflare|R2/);
  // Fields in declaration order, typed by kind; help rides the ? mark.
  assert.match(html, /API URL[\s\S]*Access Key ID[\s\S]*Region hint[\s\S]*API Secret/);
  assert.match(html, /<input type="url"[^>]*placeholder="https:\/\/example\.test\/bucket"/);
  assert.match(html, /<input type="text"[^>]*required=""/);
  assert.equal((html.match(/class="field-help"/g) ?? []).length, 2);
  assert.match(html, /aria-label="Copy it from the provider console\."/);
  assert.match(html, /class="storage-provider-url-field"/);
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
      editsExisting={false}
      onChange={() => undefined}
    />,
  );

  assert.match(html, /<label class="sync-folder-field compact"><span class="sync-field-name">Folder<\/span><div><input[^>]*\/tmp\/storage[^>]*><button[^>]*>.*Browse…<\/button><\/div><small>An absolute path on this Mac\.<\/small><\/label>/);
  assert.match(html, /role="radio" aria-checked="true" class="active"[^>]*>Local folder</);
  assert.doesNotMatch(html, /storage-setup-route-row|sync-storage-fields|type="password"|Manual setup/);
});

test("a stored secret is reported by presence only and is never rendered", () => {
  const html = renderToStaticMarkup(
    <StorageEditor
      descriptor={cloudProvider}
      form={{ name: "Cloud storage", settings: { api_url: "https://example.test/team", access_key_id: "key-id" }, typedSecrets: {} }}
      secretPresence={{ api_secret: true }}
      disabled={false}
      editsExisting={true}
      onChange={() => undefined}
    />,
  );

  // An edit draft keeps the name in the page heading and opens on the
  // configured guide row instead of the setup routes.
  assert.match(html, /sync-storage-editor-title">Storage settings</);
  assert.doesNotMatch(html, /aria-label="Storage name"|storage-setup-route-row/);
  assert.match(html, /Configured\. You can review or update the connection values below\./);
  // Public values stay editable; the secret says only that it exists.
  assert.match(html, /value="https:\/\/example\.test\/team"/);
  assert.match(html, /value="key-id"/);
  assert.match(html, /Stored — type to replace/);
  const secretInput = html.match(/<input type="password"[^>]*>/)?.[0] ?? "";
  assert.ok(secretInput, "the secret field renders as a password input");
  // Controlled, so it carries an empty value attribute — never a stored one.
  assert.doesNotMatch(secretInput, /value="[^"]+"/);
  assert.doesNotMatch(secretInput, /required/);
  assert.match(html, /aria-pressed="false"[^>]*>Show</);
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

test("agent setup reconciliation adopts only a newer revision of the exact draft", () => {
  const saved = cloudDraft({ revision: 4, name: " Team backup ", settings: {}, secrets: [{ secret_id: "api_secret", present: false }] });
  const marker = createAgentSetupMarker(saved, saved.name);
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
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, [sameNameOtherDraft, prepared]), {
    kind: "adopt",
    draft: prepared,
  });
  // Only the exact draft, only a newer revision.
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, []), { kind: "preserve", reason: "no_update" });
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, [cloudDraft({ revision: 4 })]), { kind: "preserve", reason: "no_update" });
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, [sameNameOtherDraft]), { kind: "preserve", reason: "no_update" });
  // A newer revision that changed identity, was renamed, or is incomplete
  // is refused rather than adopted.
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, [cloudDraft({ revision: 5, storage_id: "storage-changed" })]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, [cloudDraft({ revision: 5, name: "Renamed" })]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, [cloudDraft({ revision: 5, secrets: [{ secret_id: "api_secret", present: false }] })]), { kind: "preserve", reason: "invalid" });
  assert.deepEqual(reconcileAgentSetup(marker, cloudProvider, [cloudDraft({ revision: 5, settings: { api_url: "" } })]), { kind: "preserve", reason: "invalid" });
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
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Revision<\/span><b>3<\/b>/);
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
  assert.match(html, /sync-storage-repo-chip[^>]*><span>Revision<\/span><b>10<\/b>/);
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
  assert.match(html, /landing-picker-card-location">Unsaved Cloud A settings — not applied yet</);
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
      editsExisting={false}
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
      editsExisting={false}
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
      editsExisting={false}
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
    fields: [{ field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true, immutable_on_edit: true }],
    secrets: [
      { secret_id: "access_token", label: "Access token", required: true },
      { secret_id: "refresh_token", label: "Refresh token", required: true },
    ],
    setup_assistant: { label: "Set up with an agent", description: "Approve Dropbox access. The agent creates a Mallard shared folder.", managed_secret_id: "refresh_token" },
    setup_guide_url: "https://www.dropbox.com/developers/reference/auth-types",
    manual_setup_hint: "Use an existing shared or team namespace.",
};

test("Dropbox setup uses the R2 agent route and keeps two manual connection fields", () => {
  const descriptor = dropboxProvider;
  const form = { name: "Dropbox storage 1", settings: { namespace_id: "shared-42" }, typedSecrets: {} };
  const render = (editsExisting: boolean) => renderToStaticMarkup(
    <StorageEditor descriptor={descriptor} form={form} secretPresence={{ access_token: true }}
      disabled={false} editsExisting={editsExisting} onChange={() => {}}
      agentSlot={<button type="button">Run with Codex</button>} />,
  );
  const fresh = render(false);
  assert.match(fresh, /Namespace ID/);
  assert.match(fresh, /Access token/);
  assert.doesNotMatch(fresh, /Refresh token/);
  assert.match(fresh, /Setup guide/);
  assert.match(fresh, /Use an existing shared or team namespace/);
  assert.match(fresh, /Set up with an agent/);
  assert.match(fresh, /Run with Codex/);
  assert.doesNotMatch(fresh, /readOnly/);
  assert.equal(fieldsSet(descriptor, form, { access_token: true }).total, 2);
  assert.equal(storageDraftReady(descriptor, form, { access_token: true }), true);
  const edited = render(true);
  assert.match(edited, /readOnly=""[^>]*value="shared-42"/);
  assert.doesNotMatch(edited, /type="password"[^>]*readOnly/);
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
  for (const editsExisting of [false, true]) {
    const editorForm = editsExisting ? formFromDraft(dropboxProvider, {
      ...draft, secrets: draft.secrets.map(({ secret_id, present }) => ({ secret_id, present })),
    }) : form;
    const html = renderToStaticMarkup(<StorageEditor descriptor={dropboxProvider} form={editorForm} secretPresence={presence}
      disabled={false} editsExisting={editsExisting} onChange={() => {}}
      agentSlot={<button type="button">Run with Codex</button>} />);
    // Presence alone never says "Connected": that word waits for an
    // observed success on the transport.
    assert.match(html, /Access renews automatically\. Test the connection to verify it/);
    assert.doesNotMatch(html, /Connected\./);
    assert.match(html, /readOnly=""[^>]*value="shared-42"/);
    assert.match(html, /Run with Codex/);
    assert.match(html, /Refresh token/);
    assert.doesNotMatch(html, /Access token/);
    const credentialInput = html.match(/<input type="password"[^>]*>/)?.[0] ?? "";
    assert.match(credentialInput, /placeholder="Stored — type to replace"/);
    assert.match(credentialInput, editsExisting ? /value=""/ : /value="refresh-review-canary"/);
    assert.doesNotMatch(credentialInput, /readOnly|required=""/);
    assert.match(html, /sync-secret-reveal[^>]*>Show<\/button>/);
    assert.doesNotMatch(html, />Stored<|Manual setup/);
  }
  assert.deepEqual(secretOperationsFromForm(form, presence), {}, "viewing a saved token does not replace it");
  assert.deepEqual(secretOperationsFromForm({ ...form, typedSecrets: { refresh_token: "replacement-canary" } }, presence), {
    refresh_token: { op: "replace", value: "replacement-canary" },
  });
});

test("the guide row and credential field say what the transport last observed", () => {
  const draft = cloudDraft({ provider: "dropbox", provider_label: "Dropbox", name: "Dropbox storage 1",
    settings: { namespace_id: "shared-42" }, edits_existing: true,
    secrets: [{ secret_id: "access_token", present: false }, { secret_id: "refresh_token", present: true }],
  });
  const form = formFromDraft(dropboxProvider, draft);
  const presence = secretPresenceOf(draft);
  const render = (connection: StorageConnectionHealth | null) => renderToStaticMarkup(
    <StorageEditor descriptor={dropboxProvider} form={form} secretPresence={presence}
      disabled={false} editsExisting onChange={() => {}} connection={connection}
      agentSlot={<button type="button">Reconnect with Codex</button>} />,
  );
  const unknown = render(null);
  assert.match(unknown, /role="note"/);
  assert.match(unknown, /Access renews automatically\. Test the connection to verify it, or use the agent to reconnect\./);
  assert.doesNotMatch(unknown, /Connected\./);
  assert.match(unknown, /placeholder="Stored — type to replace"/);

  const connected = render({ state: "connected", code: "connected", message: "Connected.", observed_at: 1 });
  assert.match(connected, /data-icon="check-circle"/);
  assert.match(connected, /Connected\. Access renews automatically\./);

  const refused = render({
    state: "reconnect_required", code: "dropbox_reconnect_required", observed_at: 2,
    message: "Dropbox rejected the saved authorization. Reconnect Dropbox to restore access.",
    evidence: { endpoint: "oauth2/token", http_status: 400, error_tag: "invalid_grant", request_id: "req-1" },
  });
  assert.match(refused, /role="alert"/);
  assert.match(refused, /data-icon="alert-triangle"/);
  assert.match(refused, /storage-setup-guide-error">Dropbox rejected the saved authorization\. Reconnect Dropbox to restore access\.</);
  assert.match(refused, /placeholder="Rejected by Dropbox — paste a replacement"/);
  assert.match(refused, /Reconnect with Codex/);
  assert.doesNotMatch(refused, /Connected|req-1/);

  const expired = render({ state: "token_expired", code: "dropbox_token_expired", message: "The Dropbox access token expired.", observed_at: 3 });
  assert.match(expired, /placeholder="Expired — paste a replacement"/);
  // A temporary failure suspends nothing and changes no copy.
  const unreachable = render({ state: "unreachable", code: "dropbox_request_failed", message: "Dropbox could not be reached.", observed_at: 4 });
  assert.match(unreachable, /role="note"/);
  assert.doesNotMatch(unreachable, /could not be reached/);
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
    assert.match(readFileSync(path, "utf8"), /onOpenStorage\(storage\.storage_id, connectionSuspends\(storage\.connection\)\)/);
  }
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /const agentVerb = creatingStorage \? "Run with" : "Reconnect with";/);
  assert.match(workspace, /connectionSuspends\(storage\.connection\) \? \([\s\S]*?onClick=\{\(\) => onOpenStorage\(storage\.storage_id, true\)\}[\s\S]*?Reconnect \{storage\.provider_label\}/);
  assert.match(workspace, /key: "storage-connection",[\s\S]*?message: connectionDetail\(storageConnection\)/);
  assert.match(workspace, /connection=\{creatingStorage \? null : storageConnection\}/);
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /listen<StorageConnectionChangedEvent>\(\s*"project-sync-storage-connection-changed"/);
});

test("Dropbox agent watcher adopts only a complete exact draft with managed credentials", () => {
  const start = cloudDraft({ provider: "dropbox", provider_label: "Dropbox", name: "Dropbox storage 1", settings: {}, secrets: [], revision: 1 });
  const marker = createAgentSetupMarker(start, start.name);
  const ready = { ...start, revision: 2, settings: { namespace_id: "shared-42" }, secrets: [{ secret_id: "refresh_token", present: true }] };
  assert.equal(reconcileAgentSetup(marker, dropboxProvider, [ready]).kind, "adopt");
  assert.equal(reconcileAgentSetup(marker, dropboxProvider, [{ ...ready, secrets: [] }]).kind, "preserve");
  assert.equal(reconcileAgentSetup(marker, dropboxProvider, [{ ...ready, draft_id: "another-draft" }]).kind, "preserve");
  assert.equal(reconcileAgentSetup(marker, dropboxProvider, [{ ...ready, revision: 1 }]).kind, "preserve");
});

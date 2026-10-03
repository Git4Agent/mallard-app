import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "../src/components/Icons";
import Notice, { FieldMessage } from "../src/components/notice/Notice";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import NewProjectFolderStep from "../src/components/project-sync/NewProjectFolderStep";
import ProjectFolderField from "../src/components/project-sync/ProjectFolderField";
import ProjectSetupWorkspace from "../src/components/project-sync/ProjectSetupWorkspace";
import type { FolderVerdict } from "../src/components/project-sync/projectFolder";
import type {
  AgentFolder,
  ManagedCloudRepository,
  ProjectSetupDraft,
  ProviderDescriptor,
  ProviderProfileSummary,
  SetupDraftInspection,
  StorageSummary,
} from "../src/types";
import "../src/App.css";

/**
 * Mockup, round 2 of "a project from a Mallard Cloud link": direction B (a
 * From Mallard Cloud door on New project), redrawn only from the bands the
 * setup page already has — the Repository band's address picker, the
 * Storage band's pills, the Project band's Name + Folder, the Agent cards,
 * the footer's status line and the page's blocking Notice — and three ways
 * to decide the account when several Mallard Cloud storages exist:
 *
 *   M1 · Account first   the Storage band leads, as the setup page's order
 *                        has it; with several accounts none is checked until
 *                        the person picks one (setup's rule today), then the
 *                        address is looked up with it.
 *   M2 · The link picks  the address leads; Mallard looks it up with every
 *                        Mallard Cloud account on that service, checks the
 *                        one that can see it, and asks only when two can.
 *   M3 · On the row      no Storage band; the found row names the account,
 *                        and becomes a choice only when two can see it.
 *
 * Two frames render today's real pages beside them (New project, and the
 * setup page on a Mallard Cloud storage) as the style reference. Static data,
 * generic names. `npm run dev` (or `tauri dev`), then
 * /mockups/cloud-project-accounts-mockup.html (`?dir=all|today|m1|m2|m3&storages=one|several|none&access=personal|work|both|neither&kind=github|other|plain&phase=…&fail=auth|git|network&agents=default|ambiguous&folder=one|two&theme=dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Dir = "today" | "m1" | "m2" | "m3";
type Storages = "one" | "several" | "none";
type Access = "personal" | "work" | "both" | "neither";
type Kind = "github" | "other" | "plain";
type Phase = "empty" | "found" | "exists" | "checkout" | "running" | "failed" | "done";
type Fail = "auth" | "git" | "network";
type Agents = "default" | "ambiguous";
/** The Project band's destination: the shipped Folder field (Browse picks the
 *  parent and keeps the name), or Parent folder + Folder name. */
type FolderShape = "one" | "two";
interface Knobs {
  dir: Dir | "all";
  storages: Storages;
  access: Access;
  kind: Kind;
  phase: Phase;
  fail: Fail;
  agents: Agents;
  folder: FolderShape;
  theme: "light" | "dark";
}

const PHASES: [Phase, string][] = [
  ["empty", "Open"], ["found", "Found"], ["exists", "Folder taken"], ["checkout", "Already cloned"],
  ["running", "Getting"], ["failed", "Failed"], ["done", "Connected"],
];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    dir: pick("dir", ["all", "today", "m1", "m2", "m3"] as const, "all"),
    storages: pick("storages", ["one", "several", "none"] as const, "several"),
    access: pick("access", ["personal", "work", "both", "neither"] as const, "work"),
    kind: pick("kind", ["github", "other", "plain"] as const, "github"),
    phase: pick("phase", PHASES.map(([phase]) => phase), "found"),
    fail: pick("fail", ["auth", "git", "network"] as const, "auth"),
    agents: pick("agents", ["default", "ambiguous"] as const, "default"),
    folder: pick("folder", ["one", "two"] as const, "one"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/* ——— data ——————————————————————————————————————————————————————— */

const HOME = "/Users/you";
const HOST = "mallard-ai.com";
const FINGERPRINT = "f".repeat(64);

type AccountId = "personal" | "work" | "staging";
interface Account { id: AccountId; storageId: string; name: string; login: string; host: string; role: string }
const PERSONAL: Account = { id: "personal", storageId: "storage-personal", name: "Mallard Cloud Rika321", login: "Rika321", host: HOST, role: "Member" };
const WORK: Account = { id: "work", storageId: "storage-work", name: "Mallard Cloud rika-acme", login: "rika-acme", host: HOST, role: "Admin" };
const STAGING: Account = { id: "staging", storageId: "storage-staging", name: "Mallard Cloud staging", login: "Rika321", host: `staging.${HOST}`, role: "Owner" };

const storageOf = (account: Account): StorageSummary => ({
  storage_id: account.storageId,
  name: account.name,
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: account.login, is_path: false },
    { field_id: "service_origin", label: "Service", value: `https://${account.host}`, is_path: false },
  ],
  settings: { service_origin: `https://${account.host}`, account_id: `user_${account.id}`, account_login: account.login },
  secrets: [{ secret_id: "account_token", present: true }],
});

interface Repo { owner: string; name: string; git: boolean; source: string | null; host: string | null; command: string | null }
const REPO: Record<Kind, Repo> = {
  github: { owner: "maya", name: "portal", git: true, source: "github.com/acme/portal", host: "github.com", command: "gh repo clone acme/portal ~/code/portal" },
  other: { owner: "maya", name: "billing-svc", git: true, source: "git@gitlab.com:acme/billing-svc.git", host: "gitlab.com", command: "git clone git@gitlab.com:acme/billing-svc.git ~/code/billing-svc" },
  plain: { owner: "maya", name: "field-notes", git: false, source: null, host: null, command: null },
};
const addressOf = (repo: Repo) => `${repo.owner}/${repo.name}`;

interface Row { address: string; git: boolean; visibility: string; role: string; when: string; account: Account; added?: boolean; current?: boolean }
function rowsFor(accounts: Account[], seers: Account[], repo: Repo): Row[] {
  const rows: Row[] = [];
  for (const account of accounts) {
    if (seers.includes(account)) {
      rows.push({ address: addressOf(repo), git: repo.git, visibility: "Private", role: account.role, when: "2h ago", account, current: true });
    }
    if (account.id === "personal") {
      rows.push({ address: "Rika321/HorrorGame", git: true, visibility: "Private", role: "Owner", when: "Yesterday", account, added: true });
      rows.push({ address: "Rika321/notes", git: false, visibility: "Private", role: "Owner", when: "Sep 20", account });
    }
    if (account.id === "work") {
      rows.push({ address: "rika-acme/infra", git: true, visibility: "Private", role: "Owner", when: "Sep 22", account });
    }
  }
  return rows;
}

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const profile = (profileId: string, provider: string, dir: string): ProviderProfileSummary => ({
  profile_id: profileId, provider, display_name: `~/${dir}`, path: `${HOME}/${dir}`, canonical_path: `${HOME}/${dir}`,
  revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
});
const PROFILES: ProviderProfileSummary[] = [
  profile("profile-codex", "codex", ".codex"),
  profile("profile-claude", "claude", ".claude"),
  profile("profile-claude-work", "claude", ".claude-work"),
];

/* ——— today's pages, on a mocked backend ————————————————————————— */

const now = Math.floor(Date.now() / 1000);
const FOLDERS: AgentFolder[] = [
  ["code/mallard-app", 7_200, true], ["code/api-server", 86_400, true], ["notes", 259_200, false], ["code/landing-page", 604_800, true],
].map(([rel, ago, git]) => ({
  path: `${HOME}/${rel}`,
  providers: ["codex", "claude"],
  last_active: now - Number(ago),
  probe: { kind: "folder", canonical_path: `${HOME}/${rel}`, is_git_repository: Boolean(git), existing_draft_id: null, existing_project_ids: [] },
}));

const DRAFT: ProjectSetupDraft = {
  schema: 1,
  draft_id: "draft-portal",
  local_project_id: "project-portal",
  new_bundle_id: "a".repeat(32),
  project_root: `${HOME}/code/portal`,
  canonical_project_root: `${HOME}/code/portal`,
  display_name: "portal",
  repository_fingerprint: FINGERPRINT,
  profiles: { codex: { kind: "existing", profile_id: "profile-codex" }, claude: { kind: "existing", profile_id: "profile-claude" } },
  storage: { storage_id: PERSONAL.storageId },
  repository: {
    kind: "existing", storage_id: PERSONAL.storageId, bundle_id: "b".repeat(32), display_name: "maya/portal",
    repository_fingerprint: FINGERPRINT, mismatch_acknowledged: false,
  },
  revision: 1,
  created_at: now - 60,
  updated_at: now - 60,
  last_error: null,
};
const INSPECTION: SetupDraftInspection = {
  draft: DRAFT,
  sections: (["project", "profiles", "storage", "repository"] as const).map((section) => ({ section, state: "ready" })),
  can_finalize: true,
};
const cloudRepo = (overrides: Partial<ManagedCloudRepository>): ManagedCloudRepository => ({
  repository_id: "repo_portal", owner_login: "maya", name: "portal", address: "maya/portal", description: null, star_count: 0, viewer_starred: false,
  website_url: `https://${HOST}/maya/portal`, bundle_id: "b".repeat(32), repository_class: "git_project",
  visibility: "private", role: "member", can_push: true, repository_fingerprint: FINGERPRINT, git_remote: "matches",
  github_url: "https://github.com/acme/portal",
  revision: 12, updated_at: now - 7_200, resources: 9, registration_only: false, ...overrides,
});
const LISTING: ManagedCloudRepository[] = [
  cloudRepo({}),
  cloudRepo({
    repository_id: "repo_horror", owner_login: "Rika321", name: "HorrorGame", address: "Rika321/HorrorGame",
    website_url: `https://${HOST}/Rika321/HorrorGame`, bundle_id: "c".repeat(32), role: "owner",
    repository_fingerprint: "e".repeat(64), git_remote: "differs", revision: 4, updated_at: now - 86_400,
  }),
];

(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC((command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "list_agent_folders": return { folders: FOLDERS };
    case "probe_project_folder": return {
      kind: "folder", canonical_path: String(args.projectRoot), is_git_repository: true,
      existing_draft_id: String(args.projectRoot).endsWith("/portal") ? DRAFT.draft_id : null, existing_project_ids: [],
    };
    case "get_setup_draft": return DRAFT;
    case "inspect_setup_draft": return INSPECTION;
    case "update_setup_draft": return args.draft;
    case "list_managed_cloud_repositories": return LISTING;
    case "list_remote_bundle_snapshots": return [];
    default: return null;
  }
});

/* ——— the scenario ————————————————————————————————————————————————— */

const accountsOf = (knobs: Knobs): Account[] => (
  knobs.storages === "none" ? [] : knobs.storages === "one" ? [PERSONAL] : [PERSONAL, WORK, STAGING]
);
/** The accounts on the link's service: the only ones that can resolve it. */
const onHostOf = (knobs: Knobs) => accountsOf(knobs).filter((account) => account.host === HOST);
/** The accounts that can see the repository the link names. */
const seersOf = (knobs: Knobs) => onHostOf(knobs).filter((account) => knobs.access === "both" || knobs.access === account.id);
const locked = (phase: Phase) => phase === "running" || phase === "done";
const later = (phase: Phase) => phase !== "empty" && phase !== "found";

type Mode = "m1" | "m2" | "m3";
/** The account whose answer the page shows. M1 asks first: the frames show
 *  it after the person picked Rika321 (with several accounts nothing is
 *  checked until then). M2 and M3 take the one that can see the link, and
 *  leave the choice open while two can (until the person picks). */
function chosenAccount(mode: Mode, knobs: Knobs): Account | null {
  const seers = seersOf(knobs);
  if (mode === "m1") return later(knobs.phase) ? seers[0] ?? null : accountsOf(knobs)[0] ?? null;
  if (seers.length === 1) return seers[0];
  if (seers.length > 1) return later(knobs.phase) ? seers[0] : null;
  return null;
}
/** Whether the link resolved to the repository for this mode. */
function resolvedFor(mode: Mode, knobs: Knobs): boolean {
  if (knobs.phase === "empty" || onHostOf(knobs).length === 0) return false;
  if (mode === "m1") {
    const chosen = chosenAccount(mode, knobs);
    return !!chosen && seersOf(knobs).includes(chosen);
  }
  return seersOf(knobs).length > 0;
}

function folderVerdict(knobs: Knobs): FolderVerdict {
  const repo = REPO[knobs.kind];
  if (knobs.phase === "exists") {
    return {
      tone: "warn",
      message: "This folder already holds files; a clone needs a new or empty one.",
      actions: [{ label: `Use ~/code/${repo.name}-2`, value: `~/code/${repo.name}-2` }],
    };
  }
  if (knobs.phase === "checkout") {
    return repo.git
      ? { tone: "ok", message: `Already a checkout of ${repo.source}. Mallard uses it and skips the clone.` }
      : { tone: "ok", message: "An empty folder. Mallard uses it." };
  }
  if (!repo.git) return { tone: "ok", message: "New folder. Mallard creates it; Pull fills it with the files you pick." };
  return { tone: "ok", message: knobs.kind === "github" ? `New folder. Git clones ${repo.source} into it.` : "New folder. Git clones the URL above into it." };
}

function primary(mode: Mode, knobs: Knobs): { label: string; enabled: boolean } {
  const repo = REPO[knobs.kind];
  const ready = resolvedFor(mode, knobs) && chosenAccount(mode, knobs) !== null;
  switch (knobs.phase) {
    case "running": return { label: repo.git ? "Cloning…" : "Creating…", enabled: false };
    case "done": return { label: "Finishing…", enabled: false };
    case "failed": return { label: "Try again", enabled: true };
    case "checkout": return { label: "Connect and review Pull", enabled: knobs.agents === "default" };
    default: return {
      label: repo.git ? "Clone and review Pull" : "Create and review Pull",
      enabled: ready && knobs.phase !== "exists" && knobs.agents === "default",
    };
  }
}

function footerStatus(knobs: Knobs): string | null {
  const repo = REPO[knobs.kind];
  if (knobs.phase === "running") return repo.git ? `Cloning ${repo.source} · 48%` : `Creating ~/code/${repo.name}…`;
  if (knobs.phase === "done") return "Connected · opening the Pull review";
  return null;
}

/* ——— bands, in the setup page's own markup ——————————————————————— */

const chip = (label: string, value: ReactNode, key?: string) => (
  <span key={key} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>
);

function Band({ title, className, children }: { title: string; className?: string; children: ReactNode }) {
  return (
    <section className={`sync-setup-section${className ? ` ${className}` : ""}`}>
      <div className="sync-card-heading"><div><strong>{title}</strong></div></div>
      {children}
    </section>
  );
}

/** New project's two ways in, in the New storage page's door grammar. */
function Doors({ disabled }: { disabled: boolean }) {
  const door = (icon: "folder" | "storage-mallard", title: string, detail: string, checked: boolean) => (
    <div className="storage-mode-option">
      <button type="button" className="storage-mode-door" role="radio" aria-checked={checked} disabled={disabled}>
        <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
        <span className="storage-mode-door-detail">{detail}</span>
      </button>
    </div>
  );
  return (
    <div className="storage-mode-doors" role="radiogroup" aria-label="Where the project comes from">
      {door("folder", "On this computer", "A folder you already have.", false)}
      {door("storage-mallard", "From Mallard Cloud", "A repository you pushed or were invited to.", true)}
    </div>
  );
}

/** The Storage band's pills, Mallard Cloud accounts only. */
function AccountPills({ mode, knobs }: { mode: Mode; knobs: Knobs }) {
  const chosen = chosenAccount(mode, knobs);
  const seers = seersOf(knobs);
  const answered = mode === "m2" && knobs.phase !== "empty";
  const hint = mode !== "m2" || knobs.phase !== "found" || onHostOf(knobs).length < 2 ? null
    : seers.length > 1 ? `Both accounts can open ${addressOf(REPO[knobs.kind])}. Pick the one that connects it.`
    : seers.length === 0 ? `Neither account can see ${addressOf(REPO[knobs.kind])}. Pick the one that joins with an invitation code.`
    : null;
  return (
    <>
      <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
        {accountsOf(knobs).map((account) => {
          const elsewhere = account.host !== HOST;
          const blind = answered && !elsewhere && seers.length > 0 && !seers.includes(account);
          const active = chosen === account;
          const suffix = answered && elsewhere ? ` (${account.host})` : blind ? " (can’t see it)" : "";
          return (
            <button
              key={account.id}
              type="button"
              role="radio"
              aria-checked={active}
              title={`${account.name} · ${account.login} on ${account.host}`}
              className={`setup-pill${active ? " active" : ""}`}
              disabled={locked(knobs.phase) || (answered && (elsewhere || blind))}
            >
              <Icon name="storage-mallard" size={12} />
              <span className="setup-pill-label">{account.name}{suffix}</span>
              <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
            </button>
          );
        })}
      </div>
      {hint && <small className="sync-setup-hint">{hint}</small>}
    </>
  );
}

/** M3: the found row names the account, and offers a choice only when two can see it. */
function AccountChoice({ knobs, label = "Account" }: { knobs: Knobs; label?: string }) {
  const seers = seersOf(knobs);
  const options = seers.length > 0 ? seers : onHostOf(knobs);
  const chosen = chosenAccount("m3", knobs);
  if (options.length === 1) return chip(label, options[0].login, "account");
  return (
    <span className="r2-select-shell" key="account">
      <select aria-label={label} defaultValue={chosen?.id ?? ""} disabled={locked(knobs.phase)}>
        <option value="" disabled>{label}: choose…</option>
        {options.map((account) => <option key={account.id} value={account.id}>{label}: {account.login}</option>)}
      </select>
    </span>
  );
}

/** The Repository band: the shipped address picker, without New repository
 *  and without the remote comparison (there is no checkout yet). */
function RepositoryBand({ mode, knobs }: { mode: Mode; knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  const accounts = onHostOf(knobs);
  const seers = seersOf(knobs);
  const chosen = chosenAccount(mode, knobs);
  const resolved = resolvedFor(mode, knobs);
  const fixed = locked(knobs.phase);
  const value = knobs.phase === "empty" ? "" : `https://${HOST}/${addressOf(repo)}`;
  const listed = mode === "m1" ? (chosen ? [chosen] : []) : accounts;
  const rows = rowsFor(listed, seers, repo);
  const several = listed.length > 1;
  const who = mode === "m1" ? chosen?.login ?? "you"
    : accounts.length > 1 ? accounts.map((account) => account.login).join(" or ") : accounts[0]?.login ?? "you";

  let answer: ReactNode = null;
  if (accounts.length === 0) {
    answer = (
      <div className="r2-step-list">
        <div className="storage-found-empty" role="status">
          <Icon name="storage-mallard" size={24} />
          <strong>No Mallard Cloud account on this computer yet.</strong>
          <span>Sign in with GitHub to add one for {HOST}; the address opens right after.</span>
          <span className="storage-found-empty-verbs">
            <button type="button" className="btn btn-primary"><Icon name="link" size={13} /> Sign in with GitHub</button>
          </span>
        </div>
      </div>
    );
  } else if (knobs.phase !== "empty" && !resolved) {
    answer = (
      <div className="r2-step-list">
        <div className="storage-found-empty" role="status">
          <Icon name="search" size={24} />
          <strong>Nothing at {addressOf(repo)} that {who} can see.</strong>
          <span>Check the spelling. For a private repository, enter an invitation code below. Ask the owner or an admin for the code.</span>
        </div>
      </div>
    );
  } else if (resolved) {
    answer = (
      <div className="r2-step-list">
        <div className="storage-found-list" aria-label="Repository at this address">
          <div className="storage-found-row with-verb selected">
            <span className="sync-storage-repository-icon"><Icon name={repo.git ? "git-folder" : "folder"} size={14} /></span>
            <span className="storage-found-text">
              <span className="storage-found-name">
                {addressOf(repo)}
                <code>{repo.git ? "git project" : "project"}</code>
              </span>
              <span className="storage-found-verdict ok">
                <Icon name="check-circle" size={12} /> Collaborator · {(chosen ?? seers[0]).role}
              </span>
            </span>
            <span className="storage-found-facts">
              {chip("Visibility", "Private", "visibility")}
              {knobs.kind === "github" && chip("Clone from", repo.source, "source")}
              {knobs.kind === "other" && chip("Git remote", "not on GitHub", "remote")}
              {knobs.kind === "plain" && chip("Folder", "new, empty", "folder")}
              {mode === "m3" && <AccountChoice knobs={knobs} />}
            </span>
            <span className="storage-found-verb">
              <button type="button" className="btn-link">Open on website <Icon name="external-link" size={11} /></button>
              <span className="storage-found-connected"><Icon name="check" size={12} /> Selected</span>
            </span>
          </div>
        </div>
      </div>
    );
  }

  return (
    <Band title="Repository" className="repo-connect">
      <label className="sync-folder-field compact">
        <span>Repository address</span>
        <div>
          <input value={value} readOnly disabled={fixed} spellCheck={false} placeholder={`${HOST}/owner/name`} />
          <button type="button" className="btn" disabled={!value || fixed}><Icon name="search" size={12} /> Look up</button>
        </div>
        <small>Paste the page address from {HOST}, or enter owner/name.</small>
      </label>
      {answer}
      {resolved && knobs.kind === "other" && (
        <label className="sync-folder-field compact">
          <span>Clone URL</span>
          <div><input value={repo.source ?? ""} readOnly disabled={fixed} spellCheck={false} /></div>
          <small>
            {addressOf(repo)} isn’t linked to GitHub. Paste its clone URL as <code>git remote get-url origin</code> prints
            it; Mallard checks it against the repository before cloning.
          </small>
        </label>
      )}
      {accounts.length > 0 && (
        <label className="sync-folder-field compact">
          <span>Invitation code (optional)</span>
          <div>
            <input value="" readOnly disabled={fixed} spellCheck={false} placeholder="XXXXX-XXXXX-XXXXX-XXXXX" />
            {mode === "m3" && accounts.length > 1 && <AccountChoice knobs={knobs} label="Join as" />}
            <button type="button" className="btn" disabled><Icon name="lock" size={12} /> Join repository</button>
          </div>
          <small>Paste the code from the repository owner or an admin, then click Join repository. No repository address is needed.</small>
        </label>
      )}
      {accounts.length > 0 && (
        <div className="sync-bundle-match-options repo-rows" role="radiogroup" aria-label="Repositories you belong to">
          {rows.map((row) => {
            const active = resolved && !!row.current && (chosen === null ? false : row.account === chosen);
            return (
              <button
                key={`${row.account.id}-${row.address}`}
                type="button"
                role="radio"
                aria-checked={active}
                className={`sync-bundle-match-option${active ? " active" : ""}`}
                disabled={fixed}
              >
                <span className="sync-bundle-radio"><span /></span>
                <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.git ? "git-folder" : "folder"} size={24} /></span>
                <span className="sync-bundle-match-copy">
                  <strong>{row.address}{row.added && <small className="sync-bundle-recommended">Added</small>}</strong>
                  <span>{row.visibility} · {row.role}{several ? ` · ${row.account.login}` : ""}</span>
                </span>
                <span className="sync-bundle-match-meta">{row.when}</span>
              </button>
            );
          })}
        </div>
      )}
    </Band>
  );
}

/** The Project band, as setup draws it once a repository is connected:
 *  Name follows the repository; the destination is either the shipped
 *  Folder field, or Parent folder + Folder name in the same field grammar. */
function ProjectBand({ knobs }: { knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  const fixed = locked(knobs.phase);
  const verdict = folderVerdict(knobs);
  if (knobs.folder === "one") {
    return (
      <Band title="Project">
        <div className="sync-field-grid sync-setup-project-fields">
          <label>
            <span>Name</span>
            <input value={repo.name} readOnly disabled />
          </label>
          <ProjectFolderField
            value={`~/code/${repo.name}`}
            verdict={verdict}
            disabled={fixed}
            placeholder="~/code/my-app"
            onChange={() => undefined}
            onEnter={() => undefined}
            onBrowsed={() => undefined}
          />
        </div>
        <small className="sync-setup-hint">The project name follows the connected remote repo.</small>
      </Band>
    );
  }
  const name = repo.name;
  return (
    <Band title="Project">
      <div className="sync-field-grid sync-setup-project-fields">
        <div className="sync-project-folder-field">
          <label>Parent folder</label>
          <div className="sync-profile-select-row">
            <input value="~/code" onChange={() => undefined} disabled={fixed} spellCheck={false} />
            <button type="button" className="btn" disabled={fixed}><Icon name="folder" size={13} /> Browse…</button>
          </div>
        </div>
        <label>
          <span>Folder name</span>
          <input value={name} onChange={() => undefined} disabled={fixed} aria-invalid={knobs.phase === "exists" || undefined} />
        </label>
      </div>
      <p className={`sync-setup-path-hint cpm2-destination ${verdict.tone}`}>
        <code>~/code/{name}</code> · {verdict.message}
        {verdict.actions?.map((action) => <button key={action.label} type="button" className="btn-link">{action.label}</button>)}
      </p>
      <small className="sync-setup-hint">The project’s name follows the connected remote repo.</small>
    </Band>
  );
}

/** The Agent band's cards, as setup draws them. */
function AgentBand({ knobs }: { knobs: Knobs }) {
  const fixed = locked(knobs.phase);
  return (
    <Band title="Agent">
      <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
        {PROVIDERS.map((descriptor) => {
          const options = PROFILES.filter((candidate) => candidate.provider === descriptor.key);
          const value = descriptor.key === "claude" && knobs.agents === "ambiguous" ? "" : options[0].profile_id;
          return (
            <div key={descriptor.key} className="sync-provider-card enabled">
              <label className="sync-provider-toggle setup-marked-toggle">
                <input type="checkbox" checked readOnly disabled={fixed} aria-label={`Use ${descriptor.display_name} in this project`} />
                <ProviderMark provider={descriptor.key} displayName={descriptor.display_name} size={12} />
                <strong>{descriptor.display_name}</strong>
                <span>{descriptor.profile_label}</span>
              </label>
              <div className="sync-agent-profile-list">
                <div className={`sync-agent-profile-row${value ? "" : " has-error"}`}>
                  <span className="sync-select-shell">
                    <select value={value} disabled={fixed} aria-label={`${descriptor.display_name} profile`} onChange={() => undefined}>
                      <option value="">Choose profile</option>
                      {options.map((option) => <option key={option.profile_id} value={option.profile_id}>{option.display_name}</option>)}
                      <option value="__pending">Custom path…</option>
                    </select>
                  </span>
                  <button type="button" className="btn" disabled={fixed}><Icon name="folder" size={13} /> Browse</button>
                </div>
              </div>
            </div>
          );
        })}
      </div>
      {knobs.agents === "ambiguous" && <FieldMessage>Choose a profile for Claude.</FieldMessage>}
    </Band>
  );
}

/** A failed clone: the page's own blocking Notice, above the footer. */
function FailureNotice({ knobs }: { knobs: Knobs }) {
  const repo = REPO[knobs.kind];
  if (!repo.git) {
    return (
      <Notice severity="danger" timing="blocking" title="Folder not created." className="sync-setup-system-error">
        ~/code is not writable. Choose another folder above, then Try again.
      </Notice>
    );
  }
  const copy = <button type="button" className="btn">Copy command</button>;
  if (knobs.fail === "auth") {
    return (
      <Notice severity="danger" timing="blocking" title="Clone failed." action={copy} className="sync-setup-system-error">
        {knobs.kind === "github" ? "GitHub said no; the account gh signs in with may not have access to acme/portal." : "gitlab.com refused this computer’s SSH key."}{" "}
        Clone it in Terminal, where Git can ask you to sign in, then choose that folder above: <code>{repo.command}</code>
      </Notice>
    );
  }
  if (knobs.fail === "git") {
    return (
      <Notice severity="danger" timing="blocking" title="Git isn’t installed." action={copy} className="sync-setup-system-error">
        Install Apple’s command line tools, which include Git, then Try again: <code>xcode-select --install</code>
      </Notice>
    );
  }
  return (
    <Notice severity="danger" timing="blocking" title="Clone failed." className="sync-setup-system-error">
      Couldn’t reach {repo.host}. Check the network, then Try again.
    </Notice>
  );
}

/* ——— one direction ————————————————————————————————————————————————— */

function FromCloud({ mode, knobs }: { mode: Mode; knobs: Knobs }) {
  const resolved = resolvedFor(mode, knobs);
  const showPills = accountsOf(knobs).length > 0 && (mode === "m1" || (mode === "m2" && knobs.phase !== "empty"));
  const status = footerStatus(knobs);
  const verb = primary(mode, knobs);
  const bands = mode === "m1"
    ? [
      showPills && <Band key="storage" title="Storage"><AccountPills mode={mode} knobs={knobs} /></Band>,
      <RepositoryBand key="repository" mode={mode} knobs={knobs} />,
    ]
    : [
      <RepositoryBand key="repository" mode={mode} knobs={knobs} />,
      showPills && <Band key="storage" title="Storage"><AccountPills mode={mode} knobs={knobs} /></Band>,
    ];
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section sync-project-overview">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="New project">
          <header className="sync-modal-header">
            <div><h1>New project</h1></div>
            <button type="button" className="btn btn-ghost" aria-label="Close; nothing has been saved"><Icon name="x" size={17} /></button>
          </header>
          <div className="sync-modal-body">
            <Doors disabled={locked(knobs.phase)} />
            {bands}
            {resolved && <ProjectBand knobs={knobs} />}
            {resolved && <AgentBand knobs={knobs} />}
            {knobs.phase === "failed" && <FailureNotice knobs={knobs} />}
          </div>
          <footer className="sync-modal-footer sync-setup-footer">
            {status && <span className="sync-setup-save-state" aria-live="polite">{status}</span>}
            <div>
              <button type="button" className="btn btn-ghost">Cancel</button>
              <button type="button" className="btn btn-primary" disabled={!verb.enabled}>{verb.label}</button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

const DIRECTIONS: { id: Mode; title: string; note: string }[] = [
  {
    id: "m1",
    title: "M1 · Account first",
    note: "The setup page's order: the Storage band leads with every Mallard Cloud account; with several, none is checked until you pick (setup's rule today — the frame shows Rika321 picked), and the address is looked up with it. Plain and predictable, but when the link belongs to the other account the page says “nothing that Rika321 can see” and you switch pills.",
  },
  {
    id: "m2",
    title: "M2 · The link picks the account",
    note: "The address leads. Mallard looks it up with every Mallard Cloud account on that service; the Storage band then checks the one that can see it, greys out the ones that can't (as it greys an unavailable storage), and asks only when two can. Another service's storage (staging) is never a candidate.",
  },
  {
    id: "m3",
    title: "M3 · The account on the found row",
    note: "No Storage band at all: the found row carries the account as one more fact, and becomes a choice only when two accounts can see it; the invitation field gets the same choice for Join. Fewest bands, but the account hides inside the answer.",
  },
];

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    if (key === "storages" && value !== "several" && (next.access === "work" || next.access === "both")) next.access = "personal";
    if (seersOf(next).length === 0 && later(next.phase)) next.phase = "found";
    setKnobs(next);
    const query = new URLSearchParams({
      dir: next.dir, storages: next.storages, access: next.access, kind: next.kind, phase: next.phase,
      fail: next.fail, agents: next.agents, folder: next.folder, theme: next.theme,
    });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string, disabled = false) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} disabled={disabled} onClick={() => set(key, value)}>{label}</button>
  );
  const several = knobs.storages === "several";
  const nobody = seersOf(knobs).length === 0;
  const frameKey = Object.values(knobs).join("-");
  const show = (dir: Dir) => knobs.dir === "all" || knobs.dir === dir;
  return (
    <div className="spec">
      <h1 className="spec-title">New project › From Mallard Cloud — several accounts</h1>
      <p className="spec-sub">
        B, redrawn from the setup page’s own bands so nothing new has to be learned: the Repository band’s address
        picker, the Storage band’s pills, the Project band’s Name and Folder, the Agent cards, the footer’s status line,
        and the page’s blocking Notice. The open question is the account when several Mallard Cloud storages exist.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Show</span>{seg("dir", "all", "All")}{seg("dir", "today", "Today")}{seg("dir", "m1", "M1")}{seg("dir", "m2", "M2")}{seg("dir", "m3", "M3")}</div>
        <div className="spec-group"><span>Mallard Cloud storages</span>{seg("storages", "one", "One")}{seg("storages", "several", "Rika321 + rika-acme + staging")}{seg("storages", "none", "None")}</div>
        <div className="spec-group">
          <span>Who can see the link</span>
          {seg("access", "personal", "Rika321")}{seg("access", "work", "rika-acme", !several)}{seg("access", "both", "Both", !several)}{seg("access", "neither", "Neither")}
        </div>
        <div className="spec-group"><span>Project</span>{seg("kind", "github", "Git · on GitHub")}{seg("kind", "other", "Git · not on GitHub")}{seg("kind", "plain", "No Git")}</div>
        <div className="spec-group"><span>State</span>{PHASES.map(([phase, label]) => seg("phase", phase, label, nobody && later(phase)))}</div>
        <div className="spec-group"><span>Failure</span>{seg("fail", "auth", "Refused", knobs.phase !== "failed")}{seg("fail", "git", "No Git", knobs.phase !== "failed")}{seg("fail", "network", "Offline", knobs.phase !== "failed")}</div>
        <div className="spec-group"><span>Agents</span>{seg("agents", "default", "Defaults")}{seg("agents", "ambiguous", "Two Claude configs")}</div>
        <div className="spec-group"><span>Destination</span>{seg("folder", "one", "Folder (shipped)")}{seg("folder", "two", "Parent + name")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panel spec-pick">
        <strong>Recommended: M2 — the link picks the account</strong>
        <ul>
          <li>One rule: <em>the address names the service; the account that can see the repository connects it; you choose only when two can.</em> With one account (the common case) there is nothing to choose.</li>
          <li>It reuses the Storage band as it ships: a checked pill, and greyed pills with the reason in the label, the way an unavailable storage already reads.</li>
          <li>The lookups are read-only and one per account on that service, so asking every account costs one request each and writes nothing. Another service’s storage (staging) is never asked.</li>
          <li>No account can see it: the invitation code field, with the pills choosing which account joins. No account on this computer: Sign in with GitHub, then the address opens.</li>
          <li>Tie-break by entry: arriving from a storage (right after its Finish setup, or from its page) pre-checks that storage when it can see the link, so “both can” is asked only from Add project.</li>
        </ul>
      </div>

      <div className="spec-row">
        {show("today") && (
          <>
            <div className="spec-today">
              <h2 className="spec-col-title">Today · New project</h2>
              <p className="spec-col-note">The real NewProjectFolderStep. B adds the two doors above its Folder band; On this computer keeps this page as it is.</p>
              <div className="spec-frame">
                <main className="sync-main sync-project-links-page sync-project-setup-page">
                  <section className="profile-links-section sync-project-overview">
                    <NewProjectFolderStep
                      workflowEpoch={1}
                      isWorkflowCurrent={() => true}
                      projects={[]}
                      providers={PROVIDERS}
                      busy={false}
                      onCancel={() => undefined}
                      onChoose={() => undefined}
                      onOpenProject={() => undefined}
                      profiles={PROFILES}
                      storages={[]}
                      onAddCloudAccount={() => undefined}
                      onCloudProjectDone={() => undefined}
                    />
                  </section>
                </main>
              </div>
            </div>
            <div className="spec-today">
              <h2 className="spec-col-title">Today · Setup on a Mallard Cloud storage</h2>
              <p className="spec-col-note">The real ProjectSetupWorkspace, after a manual clone and Add project: the bands B reorders. The account is the Storage band’s pill, today one choice among every storage.</p>
              <div className="spec-frame">
                <main className="sync-main sync-project-links-page sync-project-setup-page">
                  <section className="profile-links-section sync-project-overview">
                    <ProjectSetupWorkspace
                      key={knobs.storages}
                      draftId={DRAFT.draft_id}
                      workflowEpoch={1}
                      isWorkflowCurrent={() => true}
                      providers={PROVIDERS}
                      profiles={PROFILES}
                      projects={[]}
                      storages={accountsOf(knobs).map(storageOf)}
                      busy={false}
                      onClose={() => undefined}
                      onDiscard={() => undefined}
                      onFinalized={() => undefined}
                    />
                  </section>
                </main>
              </div>
            </div>
          </>
        )}
        {DIRECTIONS.filter((direction) => show(direction.id)).map((direction) => (
          <div key={direction.id}>
            <h2 className="spec-col-title">{direction.title}</h2>
            <p className="spec-col-note">{direction.note}</p>
            <div className="spec-frame"><FromCloud key={frameKey} mode={direction.id} knobs={knobs} /></div>
          </div>
        ))}
      </div>

      <h2 className="spec-h2">What changed from round 1 to match the app</h2>
      <div className="spec-panel">
        <ul>
          <li><b>Link → Repository band.</b> The shipped address picker as it is (Repository address, Look up, the found row, Invitation code, the repositories you belong to), minus New repository and minus the remote comparison, since there is no checkout yet.</li>
          <li><b>Location → Project band.</b> Name follows the repository (the setup page already says so). The destination is a knob: the shipped Folder field (one path, <code>~/code/portal</code>; Browse picks the parent and keeps the name — the pattern desktop/desktop#13971 went wrong on when the name was not kept) or Parent folder + Folder name, as you first described it, in the same field grammar.</li>
          <li><b>Finish happens here.</b> Every setup band is on this page, so its one verb clones (or creates) and connects; no hand-off to a second setup page.</li>
          <li><b>Agent chips → the Agent cards.</b> Two Claude configs show as they do in setup: Choose profile, and “Choose a profile for Claude.”</li>
          <li><b>Steps lane → the footer.</b> While it runs, the footer’s status line says what is happening; a failure is the page’s blocking Notice with Copy command; Try again is the primary. The clone’s own folder, chosen with Browse, is the way around.</li>
          <li><b>One verb, the shipped one.</b> “Connect and review Pull”, prefixed by what gets the folder: Clone and review Pull, Create and review Pull.</li>
          <li><b>New CSS:</b> the connect dialog’s found-row wrap, applied to the Repository band when the page is narrow, and (Parent + name only) the spacing of the destination line under both fields.</li>
        </ul>
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);

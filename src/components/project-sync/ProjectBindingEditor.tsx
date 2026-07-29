import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { ProjectProvider, ProviderProfile, ProviderProfileSummary } from "../../types";
import Icon from "../Icons";
import {
  compactProjectPath,
  configuredProjectProvider,
  PROJECT_PROVIDERS,
  providerLabel,
  singleProviderSelection,
} from "./model";

export interface ProjectBindingDraft {
  local_project_id?: string;
  bundle_id: string;
  project_root: string;
  profile_ids: Partial<Record<ProjectProvider, string>>;
  expected_revision?: number | null;
}

interface Props {
  title: string;
  description: string;
  binding: ProjectBindingDraft;
  busy: boolean;
  error?: string | null;
  actionLabel: string;
  profiles: ProviderProfileSummary[];
  requiredProviders?: ProjectProvider[];
  onAddProfile: (provider: ProjectProvider) => Promise<ProviderProfile | null>;
  onCancel: () => void;
  onSubmit: (binding: ProjectBindingDraft) => void;
}

export default function ProjectBindingEditor({
  title,
  description,
  binding,
  busy,
  error,
  actionLabel,
  profiles,
  requiredProviders = [],
  onAddProfile,
  onCancel,
  onSubmit,
}: Props) {
  const initialProvider = configuredProjectProvider(binding.profile_ids) ?? requiredProviders[0] ?? "codex";
  const [projectRoot, setProjectRoot] = useState(binding.project_root);
  const [selectedProvider, setSelectedProvider] = useState<ProjectProvider>(initialProvider);
  const [profileIds, setProfileIds] = useState<Partial<Record<ProjectProvider, string>>>(() => (
    singleProviderSelection(binding.profile_ids ?? {}, initialProvider)
  ));

  const chooseFolder = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) setProjectRoot(picked);
  };

  const next: ProjectBindingDraft = {
    ...binding,
    project_root: projectRoot,
    profile_ids: profileIds,
  };
  const profilesComplete = !!profileIds[selectedProvider]
    && (requiredProviders.length === 0 || requiredProviders.includes(selectedProvider));

  const addProfile = async (provider: ProjectProvider) => {
    const profile = await onAddProfile(provider);
    if (profile) setProfileIds({ [provider]: profile.profile_id });
  };

  const selectProvider = (provider: ProjectProvider) => {
    setSelectedProvider(provider);
    setProfileIds((current) => singleProviderSelection(current, provider));
  };

  const profileField = (provider: ProjectProvider, label: string) => {
    const options = profiles.filter((profile) => profile.provider === provider);
    const selected = profileIds[provider] ?? "";
    const selectedProfile = options.find((profile) => profile.profile_id === selected);
    return (
      <label>
        <span>{label} profile <small>{requiredProviders.includes(provider) ? "required" : "this machine"}</small></span>
        <div className="sync-profile-select-row">
          <select
            value={selected}
            disabled={busy}
            aria-label={`${label} profile`}
            onChange={(event) => setProfileIds(
              event.target.value ? { [provider]: event.target.value } : {},
            )}
          >
            <option value="">Choose a profile</option>
            {options.map((profile) => (
              <option key={profile.profile_id} value={profile.profile_id} disabled={!profile.available || !profile.readable}>
                {profile.display_name}{!profile.available || !profile.readable ? " (unavailable)" : ""}
              </option>
            ))}
          </select>
          <button type="button" className="btn" onClick={() => void addProfile(provider)} disabled={busy}>
            <Icon name="plus" size={13} /> Add
          </button>
        </div>
        <small title={selectedProfile?.error ?? selectedProfile?.path}>
          {selectedProfile
            ? `${compactProjectPath(selectedProfile.path)}${!selectedProfile.available || !selectedProfile.readable ? " · Unavailable" : !selectedProfile.writable ? " · Read only" : ""}`
            : `Choose the ${label} home used by this project.`}
        </small>
      </label>
    );
  };

  return (
    <div className="sync-modal-backdrop" role="presentation">
      <section className="sync-modal sync-binding-dialog" role="dialog" aria-modal="true" aria-labelledby="sync-binding-title">
        <header className="sync-modal-header">
          <div>
            <span className="sync-eyebrow">Machine-local binding</span>
            <h1 id="sync-binding-title">{title}</h1>
            <p>{description}</p>
          </div>
          <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy} aria-label="Close binding editor">
            <Icon name="x" size={17} />
          </button>
        </header>
        <div className="sync-modal-body">
          <div className="sync-binding-safety">
            <Icon name="check-circle" size={17} />
            <span>Only this machine changes. Repo identity and cloud logical paths stay the same.</span>
          </div>
          <label className="sync-folder-field">
            <span>Project checkout</span>
            <div>
              <input value={projectRoot} onChange={(event) => setProjectRoot(event.target.value)} placeholder="/path/to/project" />
              <button type="button" className="btn" onClick={() => void chooseFolder()} disabled={busy}>Choose folder</button>
            </div>
            <small>{compactProjectPath(projectRoot)} becomes the root for every project-relative task and setting.</small>
          </label>
          <div className="sync-single-provider-binding">
            <div className="sync-agent-choice" role="radiogroup" aria-label="Agent used by this project">
              {PROJECT_PROVIDERS.map((provider) => (
                <button
                  key={provider}
                  type="button"
                  role="radio"
                  aria-checked={selectedProvider === provider}
                  className={selectedProvider === provider ? "active" : undefined}
                  disabled={busy}
                  onClick={() => selectProvider(provider)}
                >
                  <strong>{providerLabel(provider)}</strong>
                  <span>{provider === "codex" ? ".codex profile" : ".claude profile"}</span>
                </button>
              ))}
            </div>
            <div className="sync-provider-home-grid single">
              {profileField(selectedProvider, providerLabel(selectedProvider))}
            </div>
          </div>
          <dl className="sync-fact-grid compact">
            <div><dt>Local project</dt><dd><code>{binding.local_project_id ?? "created after confirmation"}</code></dd></div>
            <div><dt>Repo</dt><dd><code>{binding.bundle_id}</code></dd></div>
          </dl>
          {error && <div className="sync-callout error"><Icon name="alert-triangle" size={15} /> {error}</div>}
        </div>
        <footer className="sync-modal-footer">
          <span>Absolute paths are never uploaded.</span>
          <div>
            <button type="button" className="btn" onClick={onCancel} disabled={busy}>Cancel</button>
            <button type="button" className="btn btn-primary" disabled={busy || !projectRoot.trim() || !profilesComplete} onClick={() => onSubmit(next)}>
              {busy ? "Saving…" : actionLabel}
            </button>
          </div>
        </footer>
      </section>
    </div>
  );
}

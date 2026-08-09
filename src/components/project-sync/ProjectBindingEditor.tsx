import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  ProviderDescriptor,
  ProviderKey,
  ProviderProfile,
  ProviderProfileSummary,
} from "../../types";
import Icon from "../Icons";
import { compactProjectPath } from "./model";

export interface ProjectBindingDraft {
  local_project_id?: string;
  bundle_id: string;
  project_root: string;
  profile_ids: Record<ProviderKey, string>;
  expected_revision?: number | null;
}

interface Props {
  title: string;
  description: string;
  binding: ProjectBindingDraft;
  busy: boolean;
  error?: string | null;
  actionLabel: string;
  providers: ProviderDescriptor[];
  profiles: ProviderProfileSummary[];
  onAddProfile: (provider: ProviderKey) => Promise<ProviderProfile | null>;
  onCancel: () => void;
  onSubmit: (binding: ProjectBindingDraft) => void;
}

/**
 * Machine-local binding editor. Providers come from the backend registry;
 * each registered provider is independently enabled with its own profile,
 * and at least one provider must stay enabled.
 */
export default function ProjectBindingEditor({
  title,
  description,
  binding,
  busy,
  error,
  actionLabel,
  providers,
  profiles,
  onAddProfile,
  onCancel,
  onSubmit,
}: Props) {
  const [projectRoot, setProjectRoot] = useState(binding.project_root);
  const [profileIds, setProfileIds] = useState<Record<ProviderKey, string>>(() => ({
    ...(binding.profile_ids ?? {}),
  }));

  const chooseFolder = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) setProjectRoot(picked);
  };

  const next: ProjectBindingDraft = {
    ...binding,
    project_root: projectRoot,
    profile_ids: profileIds,
  };
  const enabledProviders = providers.filter((provider) => profileIds[provider.key] !== undefined);
  const profilesComplete = enabledProviders.length > 0
    && enabledProviders.every((provider) => !!profileIds[provider.key]);

  const addProfile = async (provider: ProviderKey) => {
    const profile = await onAddProfile(provider);
    if (profile) {
      setProfileIds((current) => ({ ...current, [provider]: profile.profile_id }));
    }
  };

  const toggleProvider = (provider: ProviderKey) => {
    setProfileIds((current) => {
      if (current[provider] !== undefined) {
        const { [provider]: _removed, ...rest } = current;
        return rest;
      }
      return { ...current, [provider]: "" };
    });
  };

  const providerCard = (descriptor: ProviderDescriptor) => {
    const enabled = profileIds[descriptor.key] !== undefined;
    const options = profiles.filter((profile) => profile.provider === descriptor.key);
    const selected = enabled ? profileIds[descriptor.key] : "";
    const selectedProfile = options.find((profile) => profile.profile_id === selected);
    return (
      <div key={descriptor.key} className={`sync-provider-card${enabled ? " enabled" : ""}`}>
        <label className="sync-provider-toggle">
          <input
            type="checkbox"
            checked={enabled}
            disabled={busy || (enabled && enabledProviders.length === 1)}
            aria-label={`Use ${descriptor.display_name} in this project`}
            onChange={() => toggleProvider(descriptor.key)}
          />
          <strong>{descriptor.display_name}</strong>
          <span>{descriptor.profile_label}</span>
        </label>
        {enabled ? (
          <div className="sync-profile-select-row">
            <select
              value={selected}
              disabled={busy}
              aria-label={`${descriptor.display_name} profile`}
              onChange={(event) => setProfileIds((current) => ({
                ...current,
                [descriptor.key]: event.target.value,
              }))}
            >
              <option value="">Choose a profile</option>
              {options.map((profile) => (
                <option key={profile.profile_id} value={profile.profile_id} disabled={!profile.available || !profile.readable}>
                  {profile.display_name}{!profile.available || !profile.readable ? " (unavailable)" : ""}
                </option>
              ))}
            </select>
            <button type="button" className="btn" onClick={() => void addProfile(descriptor.key)} disabled={busy}>
              <Icon name="plus" size={13} /> Add
            </button>
          </div>
        ) : (
          <small>Not used by this project on this machine.</small>
        )}
        {enabled && (
          <small title={selectedProfile?.error ?? selectedProfile?.path}>
            {selectedProfile
              ? `${compactProjectPath(selectedProfile.path)}${!selectedProfile.available || !selectedProfile.readable ? " · Unavailable" : !selectedProfile.writable ? " · Read only" : ""}`
              : `Choose the ${descriptor.display_name} home used by this project.`}
          </small>
        )}
      </div>
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
          <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
            {providers.map((descriptor) => providerCard(descriptor))}
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

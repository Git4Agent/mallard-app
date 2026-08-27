import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  ProviderDescriptor,
  ProviderKey,
  ProviderProfileSummary,
} from "../../types";
import Icon from "../Icons";
import Notice from "../notice/Notice";
import { expandHomePath, isAbsoluteProfilePath } from "../../profilePath";
import { compactProjectPath } from "./model";
import {
  profilePathHint,
  useHomePath,
  useProfilePathProbes,
  type ProfileProbeTarget,
} from "./ProjectSetupWorkspace";

/** One agent's choice on this machine: a registered profile, or a typed path
 *  that ProjectSync registers as a profile at save time. */
export type BindingProfileSelection =
  | { kind: "existing"; profile_id: string }
  | { kind: "pending"; path: string };

export interface ProjectBindingDraft {
  local_project_id?: string;
  bundle_id: string;
  project_root: string;
  profile_ids: Record<ProviderKey, string>;
  /** An active binding is a fact sheet here: checkout and agent profiles
   *  render read-only. Only an unbound or unconfirmed project still edits. */
  locked: boolean;
  expected_revision?: number | null;
}

export interface ProjectBindingSubmission {
  local_project_id?: string;
  bundle_id: string;
  project_root: string;
  profiles: Partial<Record<ProviderKey, BindingProfileSelection>>;
  expected_revision?: number | null;
}

interface Props {
  title: string;
  binding: ProjectBindingDraft;
  busy: boolean;
  error?: string | null;
  actionLabel: string;
  providers: ProviderDescriptor[];
  profiles: ProviderProfileSummary[];
  onCancel: () => void;
  onSubmit: (submission: ProjectBindingSubmission) => void;
}

/**
 * Machine-local binding editor, in the setup page's language: the same
 * hairline-ruled bands, and the same profile row — registered profiles and a
 * Custom path… choice in one select, an inline path field, Browse, and the
 * hint judged as the path is typed. Providers come from the backend registry;
 * each registered provider is independently enabled with its own profile, and
 * at least one provider must stay enabled.
 *
 * A locked draft (active binding) renders as a fact sheet instead: the
 * checkout and each agent's profile path, nothing editable, one Done button.
 */
export default function ProjectBindingEditor({
  title,
  binding,
  busy,
  error,
  actionLabel,
  providers,
  profiles,
  onCancel,
  onSubmit,
}: Props) {
  const [projectRoot, setProjectRoot] = useState(binding.project_root);
  const [selections, setSelections] = useState<Partial<Record<ProviderKey, BindingProfileSelection | null>>>(() => {
    const initial: Partial<Record<ProviderKey, BindingProfileSelection | null>> = {};
    for (const [provider, profileId] of Object.entries(binding.profile_ids ?? {})) {
      initial[provider as ProviderKey] = profileId
        ? { kind: "existing", profile_id: profileId }
        : null;
    }
    return initial;
  });
  const homePath = useHomePath();

  // Native <dialog> + showModal() supplies focus trap, Escape, inert
  // background, and focus restoration.
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  const probeTargets = useMemo<ProfileProbeTarget[]>(() => (
    providers.flatMap((descriptor) => {
      const selection = selections[descriptor.key];
      if (selection?.kind !== "pending") return [];
      const path = expandHomePath(selection.path, homePath);
      return isAbsoluteProfilePath(path) ? [{ provider: descriptor.key, path }] : [];
    })
  ), [providers, selections, homePath]);
  const pathProbes = useProfilePathProbes(probeTargets);

  const hintFor = (descriptor: ProviderDescriptor) => {
    const selection = selections[descriptor.key];
    if (selection?.kind !== "pending") return null;
    return profilePathHint(pathProbes[descriptor.key], selection.path, homePath, descriptor, profiles);
  };

  const enabledDescriptors = providers.filter((descriptor) => selections[descriptor.key] !== undefined);
  const unchosen = enabledDescriptors.filter((descriptor) => {
    const selection = selections[descriptor.key];
    return !selection || (selection.kind === "pending" && !selection.path.trim());
  });
  const agentError = enabledDescriptors.length === 0
    ? "Turn on at least one agent to continue."
    : unchosen.length > 0
      ? `Choose a profile for ${unchosen.map((descriptor) => descriptor.display_name).join(", ")}.`
      : null;
  // A typed path speaks through its own hint line; the save gate waits for
  // the probe to answer in the success tone.
  const profilesComplete = enabledDescriptors.length > 0
    && enabledDescriptors.every((descriptor) => {
      const selection = selections[descriptor.key];
      if (!selection) return false;
      if (selection.kind === "existing") return !!selection.profile_id;
      return hintFor(descriptor)?.tone === "ok";
    });
  const canSave = !busy && !!projectRoot.trim() && profilesComplete;

  const submit = () => {
    const chosen: Partial<Record<ProviderKey, BindingProfileSelection>> = {};
    for (const descriptor of enabledDescriptors) {
      const selection = selections[descriptor.key];
      if (!selection) continue;
      chosen[descriptor.key] = selection.kind === "pending"
        ? { kind: "pending", path: expandHomePath(selection.path, homePath) }
        : selection;
    }
    onSubmit({
      local_project_id: binding.local_project_id,
      bundle_id: binding.bundle_id,
      project_root: projectRoot.trim(),
      profiles: chosen,
      expected_revision: binding.expected_revision ?? null,
    });
  };

  const toggleProvider = (provider: ProviderKey) => {
    setSelections((current) => {
      if (current[provider] !== undefined) {
        const { [provider]: _removed, ...rest } = current;
        return rest;
      }
      return { ...current, [provider]: null };
    });
  };

  const chooseProjectFolder = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) setProjectRoot(picked);
  };

  const chooseProfileFolder = async (provider: ProviderKey) => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== "string" || !picked) return;
    setSelections((current) => ({ ...current, [provider]: { kind: "pending", path: picked } }));
  };

  const profileRow = (descriptor: ProviderDescriptor) => {
    const provider = descriptor.key;
    const selection = selections[provider] ?? null;
    const options = profiles.filter((profile) => profile.provider === provider);
    const value = selection?.kind === "existing"
      ? selection.profile_id
      : selection?.kind === "pending" ? "__pending" : "";
    const path = selection?.kind === "pending" ? selection.path : "";
    const rowIncomplete = unchosen.some((candidate) => candidate.key === provider);
    const hint = selection?.kind === "pending" ? hintFor(descriptor) : null;
    const hintId = `sync-binding-path-hint-${provider}`;
    return (
      <div className={`sync-agent-profile-row${selection?.kind === "pending" ? " has-custom-path" : ""}${rowIncomplete ? " has-error" : ""}`}>
        <span className="sync-select-shell">
          <select
            value={value}
            disabled={busy}
            aria-label={`${descriptor.display_name} profile`}
            aria-invalid={rowIncomplete || undefined}
            aria-describedby={rowIncomplete && agentError ? "sync-binding-error-agent" : undefined}
            onChange={(event) => {
              const next = event.target.value;
              setSelections((current) => ({
                ...current,
                [provider]: next === "__pending"
                  ? { kind: "pending", path: "" }
                  : next ? { kind: "existing", profile_id: next } : null,
              }));
            }}
          >
            <option value="">Choose profile</option>
            {options.map((profile) => {
              const unavailable = !profile.available || !profile.readable;
              return (
                <option key={profile.profile_id} value={profile.profile_id} disabled={unavailable}>
                  {profile.display_name}{unavailable ? " (unavailable)" : ""}
                </option>
              );
            })}
            <option value="__pending">Custom path…</option>
          </select>
        </span>
        {selection?.kind === "pending" && (
          <input
            value={path}
            disabled={busy}
            aria-label={`${descriptor.display_name} custom profile path`}
            aria-invalid={hint?.tone === "bad" || undefined}
            aria-describedby={[
              rowIncomplete && agentError ? "sync-binding-error-agent" : null,
              hintId,
            ].filter(Boolean).join(" ") || undefined}
            placeholder={`${descriptor.display_name} profile path`}
            spellCheck={false}
            autoFocus
            onChange={(event) => {
              const nextPath = event.target.value;
              setSelections((current) => ({
                ...current,
                [provider]: { kind: "pending", path: nextPath },
              }));
            }}
          />
        )}
        <button type="button" className="btn" onClick={() => void chooseProfileFolder(provider)} disabled={busy}>
          <Icon name="folder" size={13} /> Browse
        </button>
        {hint && (
          /* Polite, not an alert: this changes on a debounce while the field
             still holds focus, where a described-by change alone would go
             unannounced. */
          <p id={hintId} className={`sync-setup-path-hint ${hint.tone}`} aria-live="polite">
            {hint.message}
            {hint.path && <code>{hint.path}</code>}
          </p>
        )}
      </div>
    );
  };

  return (
    <dialog
      ref={dialogRef}
      className="sync-modal sync-binding-dialog"
      aria-labelledby="sync-binding-title"
      onCancel={(event) => { if (busy) event.preventDefault(); }}
      onClose={onCancel}
    >
        <header className="sync-modal-header">
          <div>
            <span className="sync-eyebrow">Machine-local binding</span>
            <h1 id="sync-binding-title">{title}</h1>
          </div>
          <button type="button" className="btn btn-ghost" onClick={onCancel} disabled={busy} aria-label="Close binding editor">
            <Icon name="x" size={17} />
          </button>
        </header>
        <div className="sync-modal-body">
          <section className="sync-setup-section">
            <div className="sync-card-heading">
              <div>
                <strong>Project</strong>
                <span>Only this machine changes — repo identity and cloud paths stay the same.</span>
              </div>
            </div>
            <div className="sync-field-grid">
              <label>
                <span>Folder</span>
                {binding.locked ? (
                  <input value={compactProjectPath(projectRoot)} readOnly title={projectRoot} />
                ) : (
                  <div className="sync-profile-select-row">
                    <input value={compactProjectPath(projectRoot)} readOnly title={projectRoot} />
                    <button type="button" className="btn" onClick={() => void chooseProjectFolder()} disabled={busy}>
                      Choose folder
                    </button>
                  </div>
                )}
              </label>
            </div>
            <dl className="sync-fact-grid compact">
              <div><dt>Local project</dt><dd><code>{binding.local_project_id ?? "created after confirmation"}</code></dd></div>
              <div><dt>Repo</dt><dd><code>{binding.bundle_id}</code></dd></div>
            </dl>
          </section>
          <section className="sync-setup-section">
            <div className="sync-card-heading">
              <div><strong>Agents</strong></div>
            </div>
            {binding.locked ? (
              /* The profiles this project uses on this machine, as facts:
                 the name it was registered under and where it lives. */
              <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
                {providers.flatMap((descriptor) => {
                  const selection = selections[descriptor.key];
                  if (selection?.kind !== "existing") return [];
                  const profile = profiles.find((candidate) => candidate.profile_id === selection.profile_id) ?? null;
                  const markers = profile
                    ? !profile.available || !profile.readable ? " · Unavailable" : !profile.writable ? " · Read only" : ""
                    : "";
                  return [(
                    <div key={descriptor.key} className="sync-provider-card enabled">
                      <div className="sync-provider-name">
                        <strong>{descriptor.display_name}</strong>
                        <span>{descriptor.profile_label}</span>
                      </div>
                      <small title={profile?.error ?? profile?.path}>
                        {profile
                          ? <>{profile.display_name} · <code>{compactProjectPath(profile.path)}</code>{markers}</>
                          : <>Profile missing on this machine · <code>{selection.profile_id}</code></>}
                      </small>
                    </div>
                  )];
                })}
              </div>
            ) : (
              <>
                <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
                  {providers.map((descriptor) => {
                    const enabled = selections[descriptor.key] !== undefined;
                    return (
                      <div key={descriptor.key} className={`sync-provider-card${enabled ? " enabled" : ""}`}>
                        <label className="sync-provider-toggle">
                          <input
                            type="checkbox"
                            checked={enabled}
                            disabled={busy || (enabled && enabledDescriptors.length === 1)}
                            aria-label={`Use ${descriptor.display_name} in this project`}
                            onChange={() => toggleProvider(descriptor.key)}
                          />
                          <strong>{descriptor.display_name}</strong>
                          <span>{descriptor.profile_label}</span>
                        </label>
                        {enabled && (
                          <div className="sync-agent-profile-list">
                            {profileRow(descriptor)}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
                {agentError && (
                  <div id="sync-binding-error-agent" className="sync-setup-field-error">
                    <Icon name="alert-triangle" size={13} /> {agentError}
                  </div>
                )}
              </>
            )}
          </section>
          {error && (
            /* The dialog's own failure, not a field's — it borrowed the
               field-error class for the red and describes no control. Both
               call sites clear the shell's error before opening this dialog,
               so the prop is null at mount and the only value it can take is a
               save that failed. That is an event, and the save did not
               happen. */
            <Notice severity="danger" timing="blocking" className="sync-setup-system-error">
              {error}
            </Notice>
          )}
        </div>
        <footer className="sync-modal-footer">
          <div>
            {binding.locked ? (
              <button type="button" className="btn" onClick={onCancel} disabled={busy}>Done</button>
            ) : (
              <>
                <button type="button" className="btn" onClick={onCancel} disabled={busy}>Cancel</button>
                <button type="button" className="btn btn-primary" disabled={!canSave} onClick={submit}>
                  {busy ? "Saving…" : actionLabel}
                </button>
              </>
            )}
          </div>
        </footer>
    </dialog>
  );
}

import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  BundleSnapshotSummary,
  DraftProfileSelection,
  LocalProjectSummary,
  ProjectDetail,
  ProjectSetupDraft,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfileSummary,
  SetupDraftInspection,
  SetupSectionStatus,
  StorageConfig,
} from "../../types";
import Icon from "../Icons";
import { projectSyncApi } from "./api";
import {
  compactProjectPath,
  errorMessage,
  formatRelativeTime,
  projectLabel,
  repositoryFingerprintMatches,
} from "./model";

interface Props {
  draftId: string;
  workflowEpoch: number;
  isWorkflowCurrent: (workflowEpoch: number) => boolean;
  providers: ProviderDescriptor[];
  profiles: ProviderProfileSummary[];
  projects: LocalProjectSummary[];
  storages: StorageConfig[];
  busy: boolean;
  onClose: () => void;
  onDiscard: (draftId: string) => void;
  onAddStorage: () => void;
  onFinalized: (
    detail: ProjectDetail,
    workflowEpoch: number,
  ) => void;
}

type SaveState = "idle" | "saving" | "saved" | "error";

const AUTOSAVE_DELAY_MS = 700;

function sectionState(sections: SetupSectionStatus[], id: string): SetupSectionStatus | null {
  return sections.find((section) => section.section === id) ?? null;
}

function sectionBadge(section: SetupSectionStatus | null) {
  if (section?.state === "attention") {
    return (
      <span className="sync-setup-state attention" aria-label="Needs review" title="Needs review">
        <Icon name="alert-triangle" size={14} />
      </span>
    );
  }
  if (section?.state === "blocked") {
    return (
      <span className="sync-setup-state blocked" aria-label="Action required" title="Action required">
        <Icon name="alert-triangle" size={14} />
      </span>
    );
  }
  return null;
}

export default function ProjectSetupWorkspace({
  draftId,
  workflowEpoch,
  isWorkflowCurrent,
  providers,
  profiles,
  projects,
  storages,
  busy,
  onClose,
  onDiscard,
  onAddStorage,
  onFinalized,
}: Props) {
  const [draft, setDraft] = useState<ProjectSetupDraft | null>(null);
  const [inspection, setInspection] = useState<SetupDraftInspection | null>(null);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [finalizing, setFinalizing] = useState(false);
  const [remoteBundles, setRemoteBundles] = useState<BundleSnapshotSummary[]>([]);
  const [bundlesLoading, setBundlesLoading] = useState(false);
  const [bundlesError, setBundlesError] = useState<string | null>(null);
  // Providers the user has switched on for this draft. Only entries with a
  // chosen profile are persisted on the draft itself; an enabled provider
  // without a selection blocks finalization locally.
  const [enabledProviders, setEnabledProviders] = useState<ReadonlySet<ProviderKey> | null>(null);

  // Autosave bookkeeping: edits mark the draft dirty; one debounced save
  // persists the latest state and refreshes inspection afterwards.
  const dirtyRef = useRef(false);
  const savingRef = useRef(false);
  const inspectRequest = useRef(0);
  const repoAutoSelected = useRef(false);
  const mountedRef = useRef(true);
  const workflowIsActive = () => (
    mountedRef.current && isWorkflowCurrent(workflowEpoch)
  );

  const refreshInspection = async (id: string) => {
    if (!workflowIsActive()) return;
    const requestId = ++inspectRequest.current;
    try {
      const next = await projectSyncApi.inspectSetupDraft(id);
      if (requestId !== inspectRequest.current || !workflowIsActive()) return;
      setInspection(next);
    } catch (reason) {
      if (requestId === inspectRequest.current && workflowIsActive()) {
        setError(errorMessage(reason));
      }
    }
  };

  useEffect(() => {
    mountedRef.current = true;
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await projectSyncApi.getSetupDraft(draftId);
        if (cancelled || !workflowIsActive()) return;
        if (!loaded) {
          setError("This setup draft no longer exists.");
          return;
        }
        const selectedKeys = Object.keys(loaded.profiles);
        setEnabledProviders(new Set(
          selectedKeys.length > 0
            ? selectedKeys
            : providers.filter((provider) => provider.setup_default).map((provider) => provider.key),
        ));
        setDraft(loaded);
        void refreshInspection(draftId);
      } catch (reason) {
        if (!cancelled && workflowIsActive()) setError(errorMessage(reason));
      }
    })();
    return () => {
      cancelled = true;
      mountedRef.current = false;
      inspectRequest.current += 1;
    };
    // The draft page owns one draft for its lifetime.
  }, [draftId]);

  // Debounced autosave whenever an edit lands.
  useEffect(() => {
    if (!draft || !dirtyRef.current || savingRef.current || !workflowIsActive()) return;
    const timer = window.setTimeout(() => {
      void (async () => {
        if (!dirtyRef.current || savingRef.current || !workflowIsActive()) return;
        savingRef.current = true;
        dirtyRef.current = false;
        setSaveState("saving");
        try {
          const saved = await projectSyncApi.updateSetupDraft(draft);
          if (!workflowIsActive()) return;
          setSaveState("saved");
          setError(null);
          // Keep local edits that raced the save; only adopt server-owned
          // fields and the advanced revision.
          setDraft((current) => current
            ? {
              ...current,
              revision: saved.revision,
              canonical_project_root: saved.canonical_project_root,
              repository_fingerprint: saved.repository_fingerprint,
              updated_at: saved.updated_at,
            }
            : saved);
          void refreshInspection(draftId);
        } catch (reason) {
          if (workflowIsActive()) {
            setSaveState("error");
            setError(errorMessage(reason));
          }
        } finally {
          savingRef.current = false;
        }
      })();
    }, AUTOSAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [draft, draftId]);

  const edit = (mutate: (current: ProjectSetupDraft) => ProjectSetupDraft) => {
    if (!workflowIsActive()) return;
    dirtyRef.current = true;
    setSaveState("saving");
    setDraft((current) => (current ? mutate(current) : current));
  };

  const selectedStorage: StorageConfig | null = useMemo(() => {
    if (!draft?.storage) return null;
    if (draft.storage.kind === "existing") {
      const id = draft.storage.storage_id;
      return storages.find((storage) => storage.id === id) ?? null;
    }
    return draft.storage.storage;
  }, [draft?.storage, storages]);
  const storageIsPending = draft?.storage?.kind === "pending";

  // Remote repositories for the selected existing storage. Pending storage
  // is not in the saved config yet, so its repos cannot be listed; the
  // repository choice stays "new repo" until setup finishes.
  const listableStorageId = draft?.storage?.kind === "existing" ? draft.storage.storage_id : null;
  useEffect(() => {
    let cancelled = false;
    if (!workflowIsActive()) return () => { cancelled = true; };
    if (!listableStorageId) {
      setRemoteBundles([]);
      setBundlesError(null);
      setBundlesLoading(false);
      return () => { cancelled = true; };
    }
    setBundlesLoading(true);
    setBundlesError(null);
    void projectSyncApi.listRemoteBundleSnapshots(listableStorageId)
      .then((bundles) => {
        if (cancelled || !workflowIsActive()) return;
        setRemoteBundles(bundles);
      })
      .catch((reason) => {
        if (cancelled || !workflowIsActive()) return;
        setRemoteBundles([]);
        setBundlesError(errorMessage(reason));
      })
      .finally(() => {
        if (!cancelled && workflowIsActive()) setBundlesLoading(false);
      });
    return () => { cancelled = true; };
  }, [listableStorageId]);

  const repositoryFingerprint = inspection?.draft.repository_fingerprint
    ?? draft?.repository_fingerprint
    ?? null;

  // Auto-select a single exact Git match once, without overriding an
  // explicit user choice; otherwise the default stays "create new repo".
  useEffect(() => {
    if (!draft || repoAutoSelected.current || bundlesLoading) return;
    if (draft.repository.kind !== "new" || !repositoryFingerprint) return;
    const matches = remoteBundles.filter((bundle) => (
      repositoryFingerprintMatches(
        bundle.repository_fingerprint,
        repositoryFingerprint,
      )
    ));
    if (matches.length === 1 && listableStorageId) {
      repoAutoSelected.current = true;
      edit((current) => ({
        ...current,
        repository: {
          kind: "existing",
          storage_id: listableStorageId,
          bundle_id: matches[0].bundle_id,
          display_name: matches[0].display_name,
          repository_fingerprint: matches[0].repository_fingerprint ?? null,
          mismatch_acknowledged: false,
        },
      }));
    }
  }, [
    bundlesLoading,
    draft,
    listableStorageId,
    remoteBundles,
    repositoryFingerprint,
  ]);

  const sections = inspection?.sections ?? [];
  const usesExistingRepo = draft?.repository.kind === "existing";
  const exactMatches = useMemo(() => (
    remoteBundles.filter((bundle) => (
      repositoryFingerprintMatches(
        bundle.repository_fingerprint,
        repositoryFingerprint,
      )
    ))
  ), [remoteBundles, repositoryFingerprint]);

  const chooseProfileFolder = async (provider: ProviderKey) => {
    const picked = await open({ directory: true, multiple: false });
    if (!workflowIsActive() || typeof picked !== "string" || !picked) return;
    edit((current) => ({
      ...current,
      profiles: {
        ...current.profiles,
        [provider]: { kind: "pending", path: picked, display_name: "" } satisfies DraftProfileSelection,
      },
    }));
  };

  /**
   * Switch one provider on or off for this draft. Disabling drops that
   * provider's draft selection and never touches another provider's entry.
   */
  const toggleSetupProvider = (provider: ProviderKey) => {
    if (!workflowIsActive() || !enabledProviders) return;
    const enabled = enabledProviders.has(provider);
    const next = new Set(enabledProviders);
    if (enabled) {
      next.delete(provider);
    } else {
      next.add(provider);
    }
    setEnabledProviders(next);
    if (enabled && draft?.profiles[provider] !== undefined) {
      edit((current) => {
        const { [provider]: _removed, ...rest } = current.profiles;
        return { ...current, profiles: rest };
      });
    }
  };

  const changeProjectFolder = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (!workflowIsActive() || typeof picked !== "string" || !picked) return;
    edit((current) => ({ ...current, project_root: picked }));
  };

  const finalize = async () => {
    if (!draft || !workflowIsActive()) return;
    let expectedRevision = draft.revision;
    // Flush any pending edit before pinning the reviewed revision.
    if (dirtyRef.current && !savingRef.current) {
      savingRef.current = true;
      dirtyRef.current = false;
      try {
        const saved = await projectSyncApi.updateSetupDraft(draft);
        if (!workflowIsActive()) return;
        setDraft((current) => current ? { ...current, revision: saved.revision } : saved);
        expectedRevision = saved.revision;
      } catch (reason) {
        if (workflowIsActive()) setError(errorMessage(reason));
        savingRef.current = false;
        return;
      }
      savingRef.current = false;
    }
    if (!workflowIsActive()) return;
    setFinalizing(true);
    setError(null);
    try {
      const detail = await projectSyncApi.finalizeProjectSetup(draftId, expectedRevision);
      if (!workflowIsActive()) return;
      onFinalized(detail, workflowEpoch);
    } catch (reason) {
      if (!workflowIsActive()) return;
      setError(errorMessage(reason));
      // The draft may carry a recorded error and a bumped revision now.
      try {
        const reloaded = await projectSyncApi.getSetupDraft(draftId);
        if (workflowIsActive() && reloaded) setDraft(reloaded);
      } catch {
        // The original error is already shown.
      }
    } finally {
      if (workflowIsActive()) setFinalizing(false);
    }
  };

  if (!draft) {
    return (
      <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="Project setup">
        {error
          ? <div className="sync-callout error"><Icon name="alert-triangle" size={15} /> {error}</div>
          : <div className="sync-storage-repository-state"><span className="status-loader" /> Loading setup draft…</div>}
      </section>
    );
  }

  const projectSection = sectionState(sections, "project");
  const profilesSection = sectionState(sections, "profiles");
  const storageSection = sectionState(sections, "storage");
  const repositorySection = sectionState(sections, "repository");
  const working = busy || finalizing;
  const setupEnabled = enabledProviders ?? new Set<ProviderKey>();
  const incompleteProviders = providers.filter((descriptor) => {
    if (!setupEnabled.has(descriptor.key)) return false;
    const selection = draft.profiles[descriptor.key];
    return !selection || (selection.kind === "pending" && !selection.path.trim());
  });
  const profileNeedsSelection = setupEnabled.size === 0 || incompleteProviders.length > 0;
  const profileBlocked = profilesSection?.state === "blocked";
  const profileErrorMessage = setupEnabled.size === 0
    ? "Turn on at least one agent to continue."
    : incompleteProviders.length > 0
      ? `Choose a profile for ${incompleteProviders.map((descriptor) => descriptor.display_name).join(", ")}.`
      : profileBlocked
        ? profilesSection?.message
        : null;
  const canFinalize = !!inspection?.can_finalize
    && !profileNeedsSelection
    && !working
    && !bundlesLoading
    && saveState !== "saving"
    && saveState !== "error";
  const showRepositoryChoice = usesExistingRepo
    || Boolean(bundlesError)
    || remoteBundles.length > 0
    || repositorySection?.state === "blocked";

  // Configs already claimed by a project on this same folder: the composite
  // project key (folder, config) makes them invalid choices for this draft.
  const claimedProfiles = new Map<string, string>();
  for (const project of projects) {
    if (project.canonical_project_root?.toLowerCase() !== draft.canonical_project_root.toLowerCase()) continue;
    for (const profileId of Object.values(project.profile_ids ?? {})) {
      if (profileId) claimedProfiles.set(profileId, projectLabel(project));
    }
  }

  const profileRow = (descriptor: ProviderDescriptor) => {
    const provider = descriptor.key;
    const label = descriptor.display_name;
    const selection = draft.profiles[provider] ?? null;
    const options = profiles.filter((profile) => profile.provider === provider);
    const value = selection?.kind === "existing"
      ? selection.profile_id
      : selection?.kind === "pending" ? "__pending" : "";
    const path = selection?.kind === "pending" ? selection.path : "";
    const rowIncomplete = incompleteProviders.some((candidate) => candidate.key === provider);
    return (
      <div
        className={`sync-agent-profile-row${selection?.kind === "pending" ? " has-custom-path" : ""}${profileBlocked || rowIncomplete ? " has-error" : ""}`}
        key={provider}
      >
        {/* A native select cannot hold an <svg>, so the app's chevron rides a
            pseudo-element on this shell — the same construction the log
            policy fields use. */}
        <span className="sync-select-shell">
          <select
            value={value}
            disabled={working}
            aria-label={`${label} profile`}
            aria-invalid={profileBlocked || rowIncomplete || undefined}
            aria-describedby={(profileBlocked || rowIncomplete) && profileErrorMessage ? "sync-setup-error-agent" : undefined}
            onChange={(event) => {
              const next = event.target.value;
              if (next === "__pending") {
                void chooseProfileFolder(provider);
                return;
              }
              edit((current) => {
                if (!next) {
                  const { [provider]: _removed, ...rest } = current.profiles;
                  return { ...current, profiles: rest };
                }
                return {
                  ...current,
                  profiles: {
                    ...current.profiles,
                    [provider]: { kind: "existing", profile_id: next } satisfies DraftProfileSelection,
                  },
                };
              });
            }}
          >
            <option value="">Choose profile</option>
            {options.map((profile) => {
              const unavailable = !profile.available || !profile.readable;
              const claimedBy = claimedProfiles.get(profile.profile_id);
              return (
                <option key={profile.profile_id} value={profile.profile_id} disabled={unavailable || Boolean(claimedBy)}>
                  {profile.display_name}{unavailable ? " (unavailable)" : claimedBy ? ` (used by ${claimedBy})` : ""}
                </option>
              );
            })}
            <option value="__pending">Custom path…</option>
          </select>
        </span>
        {selection?.kind === "pending" && (
          <input
            value={path}
            disabled={working}
            aria-label={`${label} custom profile path`}
            aria-invalid={profileBlocked || rowIncomplete || undefined}
            aria-describedby={(profileBlocked || rowIncomplete) && profileErrorMessage ? "sync-setup-error-agent" : undefined}
            placeholder={`${label} profile path`}
            spellCheck={false}
            autoFocus
            onChange={(event) => {
              const nextPath = event.target.value;
              edit((current) => ({
                ...current,
                profiles: {
                  ...current.profiles,
                  [provider]: { kind: "pending", path: nextPath, display_name: "" },
                },
              }));
            }}
          />
        )}
        <button type="button" className="btn" onClick={() => void chooseProfileFolder(provider)} disabled={working}>
          <Icon name="folder" size={13} /> Browse
        </button>
      </div>
    );
  };

  // Field errors are described-by their controls (WCAG 3.3.1): aria-invalid
  // alone says a field is wrong without ever saying why.
  const projectErrorId = projectSection?.message && projectSection.state === "blocked" ? "sync-setup-error-project" : undefined;
  const storageErrorId = storageSection?.message && storageSection.state === "blocked" ? "sync-setup-error-storage" : undefined;
  const repositoryErrorId = repositorySection?.message && repositorySection.state === "blocked" ? "sync-setup-error-repository" : undefined;

  return (
    <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-labelledby="sync-setup-title">
      <header className="sync-modal-header">
        <div>
          <h1 id="sync-setup-title">Set up {draft.display_name || "project"}</h1>
        </div>
        <button type="button" className="btn btn-ghost" onClick={onClose} disabled={finalizing} aria-label="Close setup; the draft is kept">
          <Icon name="x" size={17} />
        </button>
      </header>

      <div className="sync-modal-body">
        {draft.last_error && (
          <div className="sync-callout warning">
            <Icon name="alert-triangle" size={15} />
            <span>The last finish attempt failed: {draft.last_error}</span>
          </div>
        )}

        {/* Project */}
        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>Project</strong></div>
            {sectionBadge(projectSection)}
          </div>
          <div className="sync-field-grid sync-setup-project-fields">
            <label>
              <span>Name</span>
              <input
                value={draft.display_name}
                disabled={working || usesExistingRepo}
                aria-describedby={projectErrorId}
                onChange={(event) => edit((current) => ({ ...current, display_name: event.target.value }))}
              />
            </label>
            <label>
              <span>Folder</span>
              <div className="sync-profile-select-row">
                <input value={compactProjectPath(draft.project_root)} readOnly title={draft.project_root} aria-describedby={projectErrorId} />
                <button type="button" className="btn" onClick={() => void changeProjectFolder()} disabled={working}>Change…</button>
              </div>
            </label>
          </div>
          {projectSection?.message && projectSection.state === "blocked" && (
            <div id={projectErrorId} className="sync-setup-field-error"><Icon name="alert-triangle" size={13} /> {projectSection.message}</div>
          )}
          {usesExistingRepo && (
            <small className="sync-setup-hint">The project name follows the connected remote repo.</small>
          )}
        </section>

        {/* Agent profile */}
        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>Agent</strong></div>
            {sectionBadge(profilesSection)}
          </div>
          <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
            {providers.map((descriptor) => {
              const enabled = setupEnabled.has(descriptor.key);
              return (
                <div key={descriptor.key} className={`sync-provider-card${enabled ? " enabled" : ""}`}>
                  <label className="sync-provider-toggle">
                    <input
                      type="checkbox"
                      checked={enabled}
                      disabled={working || (enabled && setupEnabled.size === 1)}
                      aria-label={`Use ${descriptor.display_name} in this project`}
                      onChange={() => toggleSetupProvider(descriptor.key)}
                    />
                    <strong>{descriptor.display_name}</strong>
                    <span>{descriptor.profile_label}</span>
                  </label>
                  {enabled ? (
                    <div className="sync-agent-profile-list">
                      {profileRow(descriptor)}
                    </div>
                  ) : (
                    <small>Not used by this project on this machine.</small>
                  )}
                </div>
              );
            })}
          </div>
          {profileErrorMessage && (
            <div id="sync-setup-error-agent" className="sync-setup-field-error"><Icon name="alert-triangle" size={13} /> {profileErrorMessage}</div>
          )}
        </section>

        {/* Storage */}
        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>Storage</strong></div>
            {sectionBadge(storageSection)}
          </div>
          <div className="sync-profile-select-row">
            <span className="sync-select-shell">
              <select
                value={draft.storage?.kind === "existing" ? draft.storage.storage_id : draft.storage?.kind === "pending" ? "__pending" : ""}
                disabled={working}
                aria-label="Storage"
                aria-describedby={storageErrorId}
                onChange={(event) => {
                  const next = event.target.value;
                  edit((current) => ({
                    ...current,
                    // Repository choices are storage-specific.
                    repository: { kind: "new" },
                    storage: !next || next === "__pending"
                      ? (next === "__pending" ? current.storage : null)
                      : { kind: "existing", storage_id: next },
                  }));
                }}
              >
                <option value="">Local only</option>
                {storageIsPending && selectedStorage && (
                  <option value="__pending">{selectedStorage.name} (new)</option>
                )}
                {storages.map((storage) => (
                  <option key={storage.id} value={storage.id}>
                    {storage.name || "(unnamed)"} · {storage.kind === "local" ? "local folder" : "S3"}
                  </option>
                ))}
              </select>
            </span>
            <button
              type="button"
              className="btn"
              onClick={onAddStorage}
              disabled={working || saveState === "saving" || saveState === "error"}
            >
              <Icon name="plus" size={13} /> Add storage
            </button>
          </div>
          {storageSection?.message && storageSection.state === "blocked" && (
            <div id={storageErrorId} className="sync-setup-field-error"><Icon name="alert-triangle" size={13} /> {storageSection.message}</div>
          )}
          {storageIsPending && selectedStorage && (
            <small className="sync-setup-hint">{selectedStorage.local_dir}</small>
          )}
          {bundlesLoading && (
            <small className="sync-setup-inline-status"><span className="status-loader" /> Checking repositories…</small>
          )}
        </section>

        {/* Repository */}
        {showRepositoryChoice && (
          <section className="sync-setup-section">
            <div className="sync-card-heading">
              <div><strong>Repository</strong></div>
              {sectionBadge(repositorySection)}
            </div>
            {repositorySection?.message && repositorySection.state === "blocked" && (
              <div id={repositoryErrorId} className="sync-setup-field-error"><Icon name="alert-triangle" size={13} /> {repositorySection.message}</div>
            )}
            {bundlesError ? (
              /* No radiogroup renders alongside this failure, so there is no
                 field to describe — announce it instead. */
              <div className="sync-setup-field-error" role="alert"><Icon name="alert-triangle" size={13} /> {bundlesError}</div>
            ) : !listableStorageId ? null : (
              <div className="sync-bundle-match-options" role="radiogroup" aria-label="Repo identity for this project" aria-describedby={repositoryErrorId}>
                <button
                  type="button"
                  role="radio"
                  aria-checked={!usesExistingRepo}
                  className={`sync-bundle-match-option separate${!usesExistingRepo ? " active" : ""}`}
                  onClick={() => edit((current) => ({ ...current, repository: { kind: "new" } }))}
                  disabled={working}
                >
                  <span className="sync-bundle-radio"><span /></span>
                  <span className="sync-bundle-match-copy">
                    <strong>New repository</strong>
                    <span>Keep this project separate</span>
                  </span>
                </button>
                {remoteBundles.map((bundle) => {
                  const active = usesExistingRepo
                    && draft.repository.kind === "existing"
                    && draft.repository.bundle_id === bundle.bundle_id;
                  const recommended = repositoryFingerprintMatches(
                    bundle.repository_fingerprint,
                    repositoryFingerprint,
                  )
                    && exactMatches.length === 1;
                  return (
                    <button
                      key={bundle.bundle_id}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      className={`sync-bundle-match-option${active ? " active" : ""}`}
                      onClick={() => edit((current) => ({
                        ...current,
                        repository: {
                          kind: "existing",
                          storage_id: bundle.storage_id,
                          bundle_id: bundle.bundle_id,
                          display_name: bundle.display_name,
                          repository_fingerprint: bundle.repository_fingerprint ?? null,
                          mismatch_acknowledged: false,
                        },
                      }))}
                      disabled={working}
                    >
                      <span className="sync-bundle-radio"><span /></span>
                      <span className="sync-bundle-match-copy">
                        <strong>{bundle.display_name}{recommended && <small className="sync-bundle-recommended">Git match</small>}</strong>
                        <span>{bundle.bundle_id}</span>
                      </span>
                      <span className="sync-bundle-match-meta">{formatRelativeTime(bundle.updated_at)}</span>
                    </button>
                  );
                })}
              </div>
            )}
            {usesExistingRepo && draft.repository.kind === "existing"
              && !repositoryFingerprintMatches(
                draft.repository.repository_fingerprint,
                repositoryFingerprint,
              ) && (
              <label className="sync-setup-acknowledge">
                <input
                  type="checkbox"
                  checked={draft.repository.mismatch_acknowledged}
                  disabled={working}
                  onChange={(event) => edit((current) => current.repository.kind === "existing"
                    ? { ...current, repository: { ...current.repository, mismatch_acknowledged: event.target.checked } }
                    : current)}
                />
                <span>
                  This repository could not be verified for this project. Connect it anyway, then review its stored changes from Activity.
                </span>
              </label>
            )}
          </section>
        )}

        {error && (
          <div className="sync-setup-field-error sync-setup-system-error" role="alert">
            <Icon name="alert-triangle" size={13} /> {error}
          </div>
        )}
      </div>

      <footer className="sync-modal-footer sync-setup-footer">
        {(saveState === "saving" || saveState === "error") && (
          <span className="sync-setup-save-state" aria-live="polite">
            {saveState === "saving" ? "Saving…" : "Not saved"}
          </span>
        )}
        <div>
          <button type="button" className="btn btn-ghost" onClick={() => onDiscard(draftId)} disabled={finalizing}>
            Discard
          </button>
          <button type="button" className="btn btn-primary" disabled={!canFinalize} onClick={() => void finalize()}>
            {finalizing ? "Finishing…" : selectedStorage ? "Finish" : "Finish locally"}
          </button>
        </div>
      </footer>
    </section>
  );
}

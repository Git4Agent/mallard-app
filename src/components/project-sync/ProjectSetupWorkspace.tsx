import { formatRelativeTime } from "../../clock";
import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import type {
  BundleSnapshotSummary,
  DraftProfileSelection,
  LocalProjectSummary,
  ProjectDetail,
  ProjectFolderProbe,
  ProjectSetupDraft,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfileProbe,
  ProviderProfileSummary,
  SetupDraftInspection,
  SetupSectionStatus,
  StorageSummary,
} from "../../types";
import Icon, { storageIconName } from "../Icons";
import { t as translate, useTranslation } from "../../i18n";
import { backendText } from "../../i18n/backend";
import Notice, { Acknowledgement, FieldMessage } from "../notice/Notice";
import ProviderMark from "../sync-review/ProviderMark";
import {
  expandHomePath,
  isAbsoluteProfilePath,
  profilePathHint,
  type ProfilePathProbeState as ProbeState,
} from "../../profilePath";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import {
  projectLabel,
  repositoryFingerprintMatches,
  repositoryFingerprintsRequireAcknowledgement,
} from "./model";
import ProjectFolderField from "./ProjectFolderField";
import { storageProviderName } from "./storageText";
import {
  compactFolderPath,
  folderVerdict,
  readFolderInput,
  settledProbe,
  useProjectFolderProbe,
} from "./projectFolder";
import RepositoryAddressPicker, {
  isManagedCloudStorage,
  useManagedCloudRepositories,
  type RepositorySelection,
} from "./RepositoryAddressPicker";

interface Props {
  draftId: string;
  workflowEpoch: number;
  isWorkflowCurrent: (workflowEpoch: number) => boolean;
  providers: ProviderDescriptor[];
  profiles: ProviderProfileSummary[];
  projects: LocalProjectSummary[];
  storages: StorageSummary[];
  busy: boolean;
  onClose: () => void;
  onDiscard: (draftId: string) => void;
  onFinalized: (
    detail: ProjectDetail,
    workflowEpoch: number,
  ) => void;
}

type SaveState = "idle" | "saving" | "saved" | "error";

const AUTOSAVE_DELAY_MS = 700;

/** Select only one verified Git match from the chosen storage. */
export function defaultSetupRepository(
  draft: ProjectSetupDraft,
  bundles: BundleSnapshotSummary[],
  manuallySelected: boolean,
): ProjectSetupDraft["repository"] {
  if (manuallySelected || draft.repository.kind === "existing" || !draft.storage) return draft.repository;
  const matches = bundles.filter((bundle) => (
    bundle.storage_id === draft.storage?.storage_id
    && repositoryFingerprintMatches(bundle.repository_fingerprint, draft.repository_fingerprint)
  ));
  if (matches.length !== 1) return draft.repository;
  const match = matches[0];
  return {
    kind: "existing",
    storage_id: match.storage_id,
    bundle_id: match.bundle_id,
    display_name: match.display_name,
    repository_fingerprint: match.repository_fingerprint,
    mismatch_acknowledged: false,
  };
}

/** Adopt the name derived for a changed folder without losing newer edits. */
export function mergeSavedSetupDraft(
  current: ProjectSetupDraft,
  submitted: ProjectSetupDraft,
  saved: ProjectSetupDraft,
): ProjectSetupDraft {
  const sameFolder = current.project_root === submitted.project_root;
  return {
    ...current,
    revision: saved.revision,
    updated_at: saved.updated_at,
    ...(sameFolder ? {
      canonical_project_root: saved.canonical_project_root,
      repository_fingerprint: saved.repository_fingerprint,
      display_name: current.display_name === submitted.display_name ? saved.display_name : current.display_name,
    } : {}),
  };
}

/* Shorter than the autosave: the path field is judged while you type, and a
   probe only reads the filesystem. */
const PATH_PROBE_DELAY_MS = 350;

/** The project's probe, in the shared state the hint reads. */
export type ProfilePathProbeState = ProbeState<ProviderProfileProbe>;

export interface ProfileProbeTarget {
  provider: ProviderKey;
  path: string;
}

/** The home folder with its trailing slash trimmed — resolved once, because
 *  the path fields accept `~` and the backend does not. */
export function useHomePath(): string | null {
  const [homePath, setHomePath] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    void homeDir()
      .then((resolved) => { if (!cancelled) setHomePath(resolved.replace(/[\\/]+$/, "")); })
      .catch(() => { if (!cancelled) setHomePath(null); });
    return () => { cancelled = true; };
  }, []);
  return homePath;
}

/**
 * Debounced probes for every pending custom path, keyed by provider. A typed
 * path is judged as it is typed: the save-time probe is the same call, so the
 * row can report the save-time result before save.
 */
export function useProfilePathProbes(
  targets: ProfileProbeTarget[],
  isActive: () => boolean = () => true,
): Record<string, ProfilePathProbeState> {
  const [probes, setProbes] = useState<Record<string, ProfilePathProbeState>>({});
  const requestRef = useRef(0);
  const targetsRef = useRef(targets);
  targetsRef.current = targets;
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  const fingerprint = targets
    .map((target) => `${target.provider}=${target.path}`)
    .join(" ");
  useEffect(() => {
    const current = targetsRef.current;
    if (current.length === 0) {
      setProbes((existing) => (Object.keys(existing).length === 0 ? existing : {}));
      return;
    }
    const requestId = ++requestRef.current;
    setProbes((existing) => {
      const next = { ...existing };
      for (const target of current) next[target.provider] = { status: "checking" };
      return next;
    });
    const timer = window.setTimeout(() => {
      void Promise.all(current.map(async (target) => {
        try {
          const probe = await projectSyncApi.probeProviderProfile(target.provider, target.path);
          return [target.provider, { status: "ok", probe } as ProfilePathProbeState] as const;
        } catch (reason) {
          return [
            target.provider,
            { status: "error", message: errorMessage(reason) } as ProfilePathProbeState,
          ] as const;
        }
      })).then((results) => {
        if (requestRef.current !== requestId || !isActiveRef.current()) return;
        setProbes(Object.fromEntries(results));
      });
    }, PATH_PROBE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [fingerprint]);
  return probes;
}

function sectionState(sections: SetupSectionStatus[], id: string): SetupSectionStatus | null {
  return sections.find((section) => section.section === id) ?? null;
}

function sectionBadge(section: SetupSectionStatus | null) {
  if (section?.state === "attention") {
    const label = translate("projectSetup:setup.needsReview");
    return (
      <span className="sync-setup-state attention" aria-label={label} title={label}>
        <Icon name="alert-triangle" size={14} />
      </span>
    );
  }
  if (section?.state === "blocked") {
    const label = translate("projectSetup:setup.actionRequired");
    return (
      <span className="sync-setup-state blocked" aria-label={label} title={label}>
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
  onFinalized,
}: Props) {
  const { t } = useTranslation(["projectSetup", "common"]);
  const [draft, setDraft] = useState<ProjectSetupDraft | null>(null);
  const [inspection, setInspection] = useState<SetupDraftInspection | null>(null);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [finalizing, setFinalizing] = useState(false);
  const [remoteBundles, setRemoteBundles] = useState<BundleSnapshotSummary[]>([]);
  const [bundlesLoading, setBundlesLoading] = useState(false);
  const [bundlesError, setBundlesError] = useState<string | null>(null);
  const repositoryManuallySelected = useRef(false);
  // Providers the user has switched on for this draft. Only entries with a
  // chosen profile are persisted on the draft itself; an enabled provider
  // without a selection blocks finalization locally.
  const [enabledProviders, setEnabledProviders] = useState<ReadonlySet<ProviderKey> | null>(null);
  const homePath = useHomePath();

  // Autosave bookkeeping: edits mark the draft dirty; one debounced save
  // persists the latest state and refreshes inspection afterwards.
  const dirtyRef = useRef(false);
  const savingRef = useRef(false);
  const inspectRequest = useRef(0);
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
          setError(t("setup.draftGone"));
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
            ? mergeSavedSetupDraft(current, draft, saved)
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

  const selectedStorage = useMemo((): StorageSummary | null => {
    if (!draft?.storage) return null;
    return storages.find((candidate) => candidate.storage_id === draft.storage?.storage_id) ?? null;
  }, [draft?.storage, storages]);
  const chosenStorageId = draft?.storage?.storage_id ?? null;
  // A Mallard Cloud storage is not listed by bundle: its repositories are
  // named by address, and the picker reads them itself.
  const managedStorage = isManagedCloudStorage(selectedStorage);
  useEffect(() => {
    let cancelled = false;
    if (!workflowIsActive()) return () => { cancelled = true; };
    if (!chosenStorageId || managedStorage) {
      setRemoteBundles([]);
      setBundlesError(null);
      setBundlesLoading(false);
      return () => { cancelled = true; };
    }
    setBundlesLoading(true);
    setBundlesError(null);
    void projectSyncApi.listRemoteBundleSnapshots(chosenStorageId)
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
  }, [chosenStorageId, managedStorage]);

  const repositoryFingerprint = inspection?.draft.repository_fingerprint
    ?? draft?.repository_fingerprint
    ?? null;
  const managedListing = useManagedCloudRepositories(
    managedStorage ? chosenStorageId : null,
    repositoryFingerprint,
    workflowIsActive,
  );

  useEffect(() => {
    if (!draft || managedStorage || bundlesLoading || bundlesError || busy || finalizing) return;
    const repository = defaultSetupRepository(draft, remoteBundles, repositoryManuallySelected.current);
    if (repository === draft.repository) return;
    edit((current) => ({ ...current, repository }));
  }, [draft, managedStorage, bundlesLoading, bundlesError, remoteBundles, busy, finalizing]);

  const probeTargets = useMemo<ProfileProbeTarget[]>(() => (
    providers.flatMap((descriptor) => {
      const selection = draft?.profiles[descriptor.key];
      if (selection?.kind !== "pending") return [];
      const path = expandHomePath(selection.path, homePath);
      return isAbsoluteProfilePath(path) ? [{ provider: descriptor.key, path }] : [];
    })
  ), [draft?.profiles, homePath, providers]);
  const pathProbes = useProfilePathProbes(probeTargets, workflowIsActive);

  // The Folder row shows the draft's folder until it is edited, and an edit
  // moves the draft only on Enter or a Browse result: every prefix of a real
  // path is a folder too, so typing must never save one.
  const [folderText, setFolderText] = useState<string | null>(null);
  const shownFolder = folderText ?? (draft ? compactFolderPath(draft.project_root, homePath) : "");
  const folderInput = readFolderInput(shownFolder, homePath);
  const folderProbeState = useProjectFolderProbe(
    draft && folderInput.kind === "path" ? folderInput.path : null,
    workflowIsActive,
  );
  const folderUncommitted = folderText !== null
    && !(folderInput.kind === "path" && folderInput.path === draft?.project_root);
  const moveFolder = (path: string, probe: ProjectFolderProbe | null) => {
    // Another draft's folder is not somewhere this one can move.
    if (probe?.kind !== "folder" || (probe.existing_draft_id && probe.existing_draft_id !== draftId)) return;
    if (path !== draft?.project_root) edit((current) => ({ ...current, project_root: path }));
    setFolderText(null);
  };
  const browsedFolder = async (picked: string) => {
    setFolderText(compactFolderPath(picked, homePath));
    try {
      const probe = await projectSyncApi.probeProjectFolder(picked);
      if (workflowIsActive()) moveFolder(picked, probe);
    } catch {
      // The field's own probe reports the failure.
    }
  };

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
        setDraft((current) => current ? mergeSavedSetupDraft(current, draft, saved) : saved);
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
      // Finishing setup registers the repo in the chosen storage; a transport
      // failure there must name the storage and the fix, not the SDK.
      setError(storageAccessErrorMessage(selectedStorage, reason));
      // The failed finalize can record an error and advance the draft revision.
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
      <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label={t("setup.region")}>
        {error
          ? (
            /* Nothing the user just did can reach this branch: the draft is
               still null, so the only writer that has run is the mount load.
               That makes the failure content this view renders with — a named
               region, not a status message. */
            <Notice
              severity="danger"
              timing="standing"
              label={t("setup.problems")}
              className="sync-setup-system-error"
            >
              {error}
            </Notice>
          )
          : <div className="sync-storage-repository-state"><span className="status-loader" /> {t("setup.loading")}</div>}
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
    ? t("agents.noneOn")
    : incompleteProviders.length > 0
      ? t("agents.chooseFor", { providers: incompleteProviders.map((descriptor) => descriptor.display_name).join(t("shared.separator")) })
      : profileBlocked
        ? profilesSection?.message
        : null;
  const canFinalize = !!inspection?.can_finalize
    && !profileNeedsSelection
    && !working
    && !bundlesLoading
    && !(managedStorage && managedListing.state === "loading")
    && saveState !== "saving"
    && saveState !== "error"
    // Finish sets up the folder the draft holds, never one still being typed.
    && !folderUncommitted;
  // The repository decision is always explicit once a storage is chosen:
  // either the user picks from the repos already in the storage, or they see
  // the statement that Finish creates a new one. No silent default.
  const showRepositoryChoice = Boolean(chosenStorageId)
    || usesExistingRepo
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

  // Repository choices are storage-specific, so changing the storage drops the
  // repository decision rather than carrying a stale one across.
  const chooseStorage = (storageId: string | null) => {
    repositoryManuallySelected.current = false;
    edit((current) => ({
      ...current,
      repository: { kind: "new" },
      storage: storageId ? { storage_id: storageId } : null,
    }));
  };

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
    const hint = selection?.kind === "pending"
      ? profilePathHint(pathProbes[provider], path, homePath, descriptor, profiles)
      : null;
    const hintId = `sync-setup-path-hint-${provider}`;
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
            aria-label={t("agents.profile", { provider: label })}
            aria-invalid={profileBlocked || rowIncomplete || undefined}
            aria-describedby={(profileBlocked || rowIncomplete) && profileErrorMessage ? "sync-setup-error-agent" : undefined}
            onChange={(event) => {
              const next = event.target.value;
              if (next === "__pending") {
                // Open the field, not the folder dialog. A pasted path, or one
                // on a volume that is not mounted yet, has no way through a
                // picker — Browse is still right there for everyone else.
                edit((current) => ({
                  ...current,
                  profiles: {
                    ...current.profiles,
                    [provider]: { kind: "pending", path: "", display_name: "" } satisfies DraftProfileSelection,
                  },
                }));
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
            <option value="">{t("agents.chooseProfile")}</option>
            {options.map((profile) => {
              const unavailable = !profile.available || !profile.readable;
              const claimedBy = claimedProfiles.get(profile.profile_id);
              return (
                <option key={profile.profile_id} value={profile.profile_id} disabled={unavailable || Boolean(claimedBy)}>
                  {unavailable ? t("shared.unavailable", { name: profile.display_name })
                    : claimedBy ? t("agents.usedBy", { name: profile.display_name, project: claimedBy })
                      : profile.display_name}
                </option>
              );
            })}
            <option value="__pending">{t("agents.customPath")}</option>
          </select>
        </span>
        {selection?.kind === "pending" && (
          /* The field is invalid when what it holds is wrong, not when it is
             merely still empty — the select carries incompleteness. */
          <input
            value={path}
            disabled={working}
            aria-label={t("agents.customPathLabel", { provider: label })}
            aria-invalid={hint?.tone === "bad" || undefined}
            aria-describedby={[
              (profileBlocked || rowIncomplete) && profileErrorMessage ? "sync-setup-error-agent" : null,
              hintId,
            ].filter(Boolean).join(" ") || undefined}
            placeholder={t("agents.pathPlaceholder", { provider: label })}
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
          <Icon name="folder" size={13} /> {t("agents.browse")}
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

  // Field errors are described-by their controls (WCAG 3.3.1): aria-invalid
  // alone says a field is wrong without ever saying why.
  const projectErrorId = projectSection?.message && projectSection.state === "blocked" ? "sync-setup-error-project" : undefined;
  const storageErrorId = storageSection?.message && storageSection.state === "blocked" ? "sync-setup-error-storage" : undefined;
  const repositoryErrorId = repositorySection?.message && repositorySection.state === "blocked" ? "sync-setup-error-repository" : undefined;

  return (
    <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-labelledby="sync-setup-title">
      <header className="sync-modal-header">
        <div>
          <h1 id="sync-setup-title">{draft.display_name ? t("setup.title", { name: draft.display_name }) : t("setup.titleUnnamed")}</h1>
        </div>
        <button type="button" className="btn btn-ghost" onClick={onClose} disabled={finalizing} aria-label={t("setup.close")}>
          <Icon name="x" size={17} />
        </button>
      </header>

      <div className="sync-modal-body">
        {draft.last_error && (
          /* The draft arrives carrying this from an earlier session — it is
             loaded with the draft rather than raised by anything the user just
             did, so it is standing content and must not announce. Nothing is
             broken now; the reader is being told the last attempt did not
             land, which is the warning rung. */
          <Notice
            severity="warning"
            timing="standing"
            label={t("setup.notes")}
          >
            {t("setup.lastError", { error: draft.last_error })}
          </Notice>
        )}

        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>{t("sections.project")}</strong></div>
            {sectionBadge(projectSection)}
          </div>
          <div className="sync-field-grid sync-setup-project-fields">
            <label>
              <span>{t("sections.name")}</span>
              <input
                value={draft.display_name}
                disabled={working || usesExistingRepo}
                aria-describedby={projectErrorId}
                onChange={(event) => edit((current) => ({ ...current, display_name: event.target.value }))}
              />
            </label>
            <ProjectFolderField
              value={shownFolder}
              verdict={folderVerdict(folderInput, folderProbeState, {
                compact: (path) => compactFolderPath(path, homePath),
                projects,
                draftId,
              })}
              disabled={working}
              placeholder="~/code/my-app"
              describedBy={projectErrorId}
              onChange={setFolderText}
              onEnter={() => {
                if (folderInput.kind === "path") {
                  moveFolder(folderInput.path, settledProbe(folderInput, folderProbeState));
                }
              }}
              onBrowsed={(picked) => void browsedFolder(picked)}
            />
          </div>
          {projectSection?.message && projectSection.state === "blocked" && (
            <FieldMessage id={projectErrorId}>{projectSection.message}</FieldMessage>
          )}
          {usesExistingRepo && (
            <small className="sync-setup-hint">{t("setup.followsRemote")}</small>
          )}
        </section>

        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>{t("sections.agent")}</strong></div>
            {sectionBadge(profilesSection)}
          </div>
          <div className="sync-provider-cards" role="group" aria-label={t("agents.group")}>
            {providers.map((descriptor) => {
              const enabled = setupEnabled.has(descriptor.key);
              return (
                <div key={descriptor.key} className={`sync-provider-card${enabled ? " enabled" : ""}`}>
                  <label className="sync-provider-toggle setup-marked-toggle">
                    <input
                      type="checkbox"
                      checked={enabled}
                      disabled={working || (enabled && setupEnabled.size === 1)}
                      aria-label={t("agents.use", { provider: descriptor.display_name })}
                      onChange={() => toggleSetupProvider(descriptor.key)}
                    />
                    <ProviderMark provider={descriptor.key} displayName={descriptor.display_name} size={12} />
                    <strong>{descriptor.display_name}</strong>
                    <span>{backendText(`agentProfile.${descriptor.key}`, descriptor.profile_label)}</span>
                  </label>
                  {enabled ? (
                    <div className="sync-agent-profile-list">
                      {profileRow(descriptor)}
                    </div>
                  ) : (
                    <small>{t("agents.notUsed")}</small>
                  )}
                </div>
              );
            })}
          </div>
          {profileErrorMessage && (
            <FieldMessage id="sync-setup-error-agent">{profileErrorMessage}</FieldMessage>
          )}
        </section>

        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>{t("sections.storage")}</strong></div>
            {/* `attention` on this section means one thing — no storage is
                linked — and the row below already says so as its own answer.
                A chosen "None yet" is not something to review, and the badge
                carried no message anyway: the page prints a section's message
                only when it is blocked, so the triangle marked the safe
                answer as a fault and would not say why. Only blocked storage
                gets a badge now. */}
            {storageSection?.state === "blocked" ? sectionBadge(storageSection) : null}
          </div>
          {/* Each storage wears the glyph its own landing card wears — the
              fact a select had no way to hold. "None yet" leads: it is the
              answer a project has until a storage is chosen, and it names the
              empty slot the way the rest of the app does ("No storage
              linked") rather than a mode competing with the names beside it.
              The glyph still says where the sessions live, so the label is
              free to say only what the slot holds. */}
          <div
            className="setup-pill-row"
            role="radiogroup"
            aria-label={t("sections.storage")}
            aria-describedby={storageErrorId}
          >
            <button
              type="button"
              role="radio"
              aria-checked={!chosenStorageId}
              className={`setup-pill${chosenStorageId ? "" : " active"}`}
              disabled={working}
              /* The siblings carry their provider here; this one carries what
                 the empty slot means, in the sentence Activity already uses
                 for the same state. "Computer", not "Mac": this build ships
                 for every desktop target. */
              title={t("setup.noneYetTitle")}
              onClick={() => chooseStorage(null)}
            >
              <Icon name="computer" size={12} />
              <span className="setup-pill-label">{t("setup.noneYet")}</span>
              <span className="setup-pill-check" aria-hidden="true">
                {!chosenStorageId && <Icon name="check" size={12} />}
              </span>
            </button>
            {storages.map((storage) => {
              const active = chosenStorageId === storage.storage_id;
              const name = storage.name || t("shared.unnamed");
              return (
                <button
                  key={storage.storage_id}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  /* The name alone is the visible label; the provider it runs
                     on stays in the accessible name and the tooltip, where the
                     select used to spell it out on every row. */
                  aria-label={`${name} · ${storageProviderName(storage)}`}
                  title={`${name} · ${storageProviderName(storage)}`}
                  className={`setup-pill${active ? " active" : ""}`}
                  disabled={working || !storage.available}
                  onClick={() => chooseStorage(storage.storage_id)}
                >
                  <Icon name={storageIconName(storage.provider, storage.icon)} size={12} />
                  <span className="setup-pill-label">{storage.available ? name : t("shared.unavailable", { name })}</span>
                  <span className="setup-pill-check" aria-hidden="true">
                    {active && <Icon name="check" size={12} />}
                  </span>
                </button>
              );
            })}
          </div>
          {storageSection?.message && storageSection.state === "blocked" && (
            <FieldMessage id={storageErrorId}>{storageSection.message}</FieldMessage>
          )}
          {bundlesLoading && (
            <small className="sync-setup-inline-status"><span className="status-loader" /> {t("setup.checkingRepositories")}</small>
          )}
        </section>

        {showRepositoryChoice && (
          <section className={`sync-setup-section${managedStorage ? " repo-connect" : ""}`}>
            <div className="sync-card-heading">
              <div><strong>{t("sections.repository")}</strong></div>
              {sectionBadge(repositorySection)}
            </div>
            {repositorySection?.message && repositorySection.state === "blocked" && (
              <FieldMessage id={repositoryErrorId}>{repositorySection.message}</FieldMessage>
            )}
            {managedStorage && selectedStorage ? (
              /* Mallard Cloud: the repository is named by its website
                 address, or picked from the rows the account belongs to; the
                 project name above is the requested name for a new one, and
                 the New row previews the address it makes. */
              <RepositoryAddressPicker
                storage={selectedStorage}
                /* i18n-ignore: the New row's address preview falls back to this
                   repository name; it is a name, not copy. */
                projectName={draft.display_name || "project"}
                localFingerprint={repositoryFingerprint}
                listing={managedListing}
                selection={draft.repository.kind === "existing"
                  ? {
                    kind: "existing",
                    bundle_id: draft.repository.bundle_id,
                    display_name: draft.repository.display_name,
                    repository_fingerprint: draft.repository.repository_fingerprint ?? null,
                    mismatch_acknowledged: draft.repository.mismatch_acknowledged,
                  }
                  : { kind: "new" }}
                rows="match-option"
                offerNew
                disabled={working}
                describedBy={repositoryErrorId}
                /* An address that names nothing leaves the draft at its own
                   default, New repository, which the rows show as selected. */
                onSelect={(selection: RepositorySelection | null) => edit((current) => ({
                  ...current,
                  repository: !selection || selection.kind === "new"
                    ? { kind: "new" }
                    : { ...selection, storage_id: selectedStorage.storage_id },
                }))}
              />
            ) : bundlesError ? (
              /* Not a field error despite the company it keeps: no radiogroup
                 renders alongside this failure, so there is no control to
                 describe. Listing the storage's repos is what choosing a
                 storage kicks off, and this is that request failing — an event
                 that leaves the section with no choice to offer, so it takes
                 the blocking rung. */
              <Notice severity="danger" timing="blocking">{bundlesError}</Notice>
            ) : !chosenStorageId || bundlesLoading ? null : (
              /* An empty storage renders the same option card, alone and
                 selected: the user sees the claim Finish is about to make in
                 the section's own language, never a silent default. */
              <div className="sync-bundle-match-options" role="radiogroup" aria-label={t("setup.repoIdentity")} aria-describedby={repositoryErrorId}>
                <button
                  type="button"
                  role="radio"
                  aria-checked={!usesExistingRepo}
                  className={`sync-bundle-match-option separate${!usesExistingRepo ? " active" : ""}`}
                  onClick={() => {
                    repositoryManuallySelected.current = true;
                    edit((current) => ({ ...current, repository: { kind: "new" } }));
                  }}
                  disabled={working}
                >
                  <span className="sync-bundle-radio"><span /></span>
                  {/* Its own line, the mark Sync already uses for a history
                      kept independently. */}
                  <span className="setup-choice-mark" aria-hidden="true"><Icon name="git-fork" size={24} /></span>
                  <span className="sync-bundle-match-copy">
                    <strong>{t("shared.newRepository")}</strong>
                    <span>
                      {remoteBundles.length === 0
                        ? selectedStorage
                          ? t("setup.createsIn", { name: draft.display_name, storage: selectedStorage.name })
                          : t("setup.createsInSelected", { name: draft.display_name })
                        : t("setup.keepSeparate")}
                    </span>
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
                      {/* Whether the stored repository is a git one, said the
                          way every other list of repositories says it. */}
                      <span className="setup-choice-mark" aria-hidden="true">
                        <Icon name={bundle.repository_fingerprint ? "git-folder" : "folder"} size={24} />
                      </span>
                      <span className="sync-bundle-match-copy">
                        <strong>{bundle.display_name}{recommended && <small className="sync-bundle-recommended">{t("setup.gitMatch")}</small>}</strong>
                        <span>{bundle.bundle_id}</span>
                      </span>
                      <span className="sync-bundle-match-meta">{formatRelativeTime(bundle.updated_at)}</span>
                    </button>
                  );
                })}
              </div>
            )}
            {!managedStorage && usesExistingRepo && draft.repository.kind === "existing"
              && repositoryFingerprintsRequireAcknowledgement(
                draft.repository.repository_fingerprint,
                repositoryFingerprint,
              ) && (
              <Acknowledgement
                checked={draft.repository.mismatch_acknowledged}
                disabled={working}
                onChange={(checked) => edit((current) => current.repository.kind === "existing"
                  ? { ...current, repository: { ...current.repository, mismatch_acknowledged: checked } }
                  : current)}
              >
                {t("shared.unverified")}
              </Acknowledgement>
            )}
          </section>
        )}

        {error && (
          <Notice severity="danger" timing="blocking" className="sync-setup-system-error">
            {error}
          </Notice>
        )}
      </div>

      <footer className="sync-modal-footer sync-setup-footer">
        {(saveState === "saving" || saveState === "error") ? (
          <span className="sync-setup-save-state" aria-live="polite">
            {saveState === "saving" ? t("common:saving") : t("common:notSaved")}
          </span>
        ) : folderUncommitted && (
          <span className="sync-setup-save-state" aria-live="polite">
            {t("setup.pressEnter")}
          </span>
        )}
        <div>
          <button type="button" className="btn btn-ghost" onClick={() => onDiscard(draftId)} disabled={finalizing}>
            {t("setup.discard")}
          </button>
          <button type="button" className="btn btn-primary" disabled={!canFinalize} onClick={() => void finalize()}>
            {finalizing ? t("setup.finishing") : selectedStorage ? t("setup.finish") : t("setup.finishLocally")}
          </button>
        </div>
      </footer>
    </section>
  );
}

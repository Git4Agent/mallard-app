import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import type {
  BundleSnapshotSummary,
  DraftProfileSelection,
  LocalProjectSummary,
  ProjectDetail,
  ProjectSetupDraft,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfileProbe,
  ProviderProfileSummary,
  SetupDraftInspection,
  SetupSectionStatus,
  StorageSummary,
} from "../../types";
import Icon, { iconNameFromHint } from "../Icons";
import Notice from "../notice/Notice";
import ProviderMark from "../sync-review/ProviderMark";
import { expandHomePath, isAbsoluteProfilePath } from "../../profilePath";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import {
  compactProjectPath,
  formatRelativeTime,
  projectLabel,
  repositoryFingerprintMatches,
  repositoryFingerprintsRequireAcknowledgement,
} from "./model";

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
/* Shorter than the autosave: the path field is judged while you type, and a
   probe only reads the filesystem. */
const PATH_PROBE_DELAY_MS = 350;

export type ProfilePathProbeState =
  | { status: "checking" }
  | { status: "ok"; probe: ProviderProfileProbe }
  | { status: "error"; message: string };

export interface ProfilePathHint {
  tone: "mute" | "ok" | "warn" | "bad";
  message: string;
  /** Rendered in the mono face when present — it is a machine identifier. */
  path?: string;
}

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

/**
 * Probe refusals are written for people ("cannot be a symlink; choose the
 * resolved directory"), so they are shown as they arrive — minus the internal
 * call prefix. The one exception is the miss that happens on every keystroke
 * of a path being typed, which deserves a sentence rather than an errno.
 */
function readableProbeError(message: string): string {
  if (/no such file or directory|cannot find the path|os error 2/i.test(message)) {
    return "No folder at this path yet.";
  }
  return message.replace(/^inspect provider profile '[^']*':\s*/, "");
}

/** What the line under a custom path field says, for every state it has. */
export function profilePathHint(
  state: ProfilePathProbeState | undefined,
  rawPath: string,
  home: string | null,
  descriptor: Pick<ProviderDescriptor, "display_name" | "default_profile_directory">,
  profiles: Array<Pick<ProviderProfileSummary, "profile_id" | "display_name">>,
): ProfilePathHint {
  const expanded = expandHomePath(rawPath, home);
  if (!expanded) {
    return { tone: "mute", message: "Type or paste a path, or Browse. ~ means your home folder." };
  }
  if (!isAbsoluteProfilePath(expanded)) {
    return { tone: "mute", message: "Paths must be absolute — start with ~/, / or a drive letter." };
  }
  if (!state || state.status === "checking") {
    return { tone: "mute", message: "Checking", path: expanded };
  }
  if (state.status === "error") {
    return { tone: "bad", message: readableProbeError(state.message) };
  }
  const { probe } = state;
  const registered = probe.existing_profile_id
    ? profiles.find((profile) => profile.profile_id === probe.existing_profile_id) ?? null
    : null;
  if (registered) {
    return { tone: "ok", message: `Already set up as “${registered.display_name}” — setup will reuse it.` };
  }
  if (!probe.readable) {
    return { tone: "warn", message: "This folder can’t be read. Check its permissions.", path: probe.canonical_path };
  }
  if (probe.detected_child && descriptor.default_profile_directory) {
    return {
      tone: "ok",
      message: `Found ${descriptor.default_profile_directory} inside — using`,
      path: probe.canonical_path,
    };
  }
  return { tone: "ok", message: `${descriptor.display_name} profile ·`, path: probe.canonical_path };
}

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
  const homePath = useHomePath();

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

  const selectedStorage = useMemo((): StorageSummary | null => {
    if (!draft?.storage) return null;
    return storages.find((candidate) => candidate.storage_id === draft.storage?.storage_id) ?? null;
  }, [draft?.storage, storages]);
  const chosenStorageId = draft?.storage?.storage_id ?? null;
  useEffect(() => {
    let cancelled = false;
    if (!workflowIsActive()) return () => { cancelled = true; };
    if (!chosenStorageId) {
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
  }, [chosenStorageId]);

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
    if (matches.length === 1 && chosenStorageId) {
      repoAutoSelected.current = true;
      edit((current) => ({
        ...current,
        repository: {
          kind: "existing",
          storage_id: chosenStorageId,
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
    chosenStorageId,
    remoteBundles,
    repositoryFingerprint,
  ]);

  const probeTargets = useMemo<ProfileProbeTarget[]>(() => (
    providers.flatMap((descriptor) => {
      const selection = draft?.profiles[descriptor.key];
      if (selection?.kind !== "pending") return [];
      const path = expandHomePath(selection.path, homePath);
      return isAbsoluteProfilePath(path) ? [{ provider: descriptor.key, path }] : [];
    })
  ), [draft?.profiles, homePath, providers]);
  const pathProbes = useProfilePathProbes(probeTargets, workflowIsActive);

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
      <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="Project setup">
        {error
          ? (
            /* Nothing the user just did can reach this branch: the draft is
               still null, so the only writer that has run is the mount load.
               That makes the failure content this view renders with — a named
               region, not a status message. */
            <Notice
              severity="danger"
              timing="standing"
              label="Problems with this setup draft"
              className="sync-setup-system-error"
            >
              {error}
            </Notice>
          )
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
  const chooseStorage = (storageId: string | null) => edit((current) => ({
    ...current,
    repository: { kind: "new" },
    storage: storageId ? { storage_id: storageId } : null,
  }));

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
            aria-label={`${label} profile`}
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
          /* The field is invalid when what it holds is wrong, not when it is
             merely still empty — the select carries incompleteness. */
          <input
            value={path}
            disabled={working}
            aria-label={`${label} custom profile path`}
            aria-invalid={hint?.tone === "bad" || undefined}
            aria-describedby={[
              (profileBlocked || rowIncomplete) && profileErrorMessage ? "sync-setup-error-agent" : null,
              hintId,
            ].filter(Boolean).join(" ") || undefined}
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
          <h1 id="sync-setup-title">Set up {draft.display_name || "project"}</h1>
        </div>
        <button type="button" className="btn btn-ghost" onClick={onClose} disabled={finalizing} aria-label="Close setup; the draft is kept">
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
            label="Notes about this setup draft"
          >
            The last finish attempt failed: {draft.last_error}
          </Notice>
        )}

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
                {/* The same glyph Browse wears two bands down: both buttons
                    open the same folder picker. */}
                <button type="button" className="btn" onClick={() => void changeProjectFolder()} disabled={working}>
                  <Icon name="folder" size={13} /> Change…
                </button>
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
                  <label className="sync-provider-toggle setup-marked-toggle">
                    <input
                      type="checkbox"
                      checked={enabled}
                      disabled={working || (enabled && setupEnabled.size === 1)}
                      aria-label={`Use ${descriptor.display_name} in this project`}
                      onChange={() => toggleSetupProvider(descriptor.key)}
                    />
                    <ProviderMark provider={descriptor.key} displayName={descriptor.display_name} size={12} />
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

        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>Storage</strong></div>
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
            aria-label="Storage"
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
              title="No storage linked yet — sessions stay on this computer"
              onClick={() => chooseStorage(null)}
            >
              <Icon name="computer" size={12} />
              <span className="setup-pill-label">None yet</span>
              <span className="setup-pill-check" aria-hidden="true">
                {!chosenStorageId && <Icon name="check" size={12} />}
              </span>
            </button>
            {storages.map((storage) => {
              const active = chosenStorageId === storage.storage_id;
              const name = storage.name || "(unnamed)";
              return (
                <button
                  key={storage.storage_id}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  /* The name alone is the visible label; the provider it runs
                     on stays in the accessible name and the tooltip, where the
                     select used to spell it out on every row. */
                  aria-label={`${name} · ${storage.provider_label}`}
                  title={`${name} · ${storage.provider_label}`}
                  className={`setup-pill${active ? " active" : ""}`}
                  disabled={working || !storage.available}
                  onClick={() => chooseStorage(storage.storage_id)}
                >
                  <Icon name={iconNameFromHint(storage.icon, "drive")} size={12} />
                  <span className="setup-pill-label">{name}{storage.available ? "" : " (unavailable)"}</span>
                  <span className="setup-pill-check" aria-hidden="true">
                    {active && <Icon name="check" size={12} />}
                  </span>
                </button>
              );
            })}
          </div>
          {storageSection?.message && storageSection.state === "blocked" && (
            <div id={storageErrorId} className="sync-setup-field-error"><Icon name="alert-triangle" size={13} /> {storageSection.message}</div>
          )}
          {bundlesLoading && (
            <small className="sync-setup-inline-status"><span className="status-loader" /> Checking repositories…</small>
          )}
        </section>

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
                  {/* Its own line, the mark Sync already uses for a history
                      kept independently. */}
                  <span className="setup-choice-mark" aria-hidden="true"><Icon name="git-fork" size={24} /></span>
                  <span className="sync-bundle-match-copy">
                    <strong>New repository</strong>
                    <span>
                      {remoteBundles.length === 0
                        ? `Creates “${draft.display_name}” in ${selectedStorage?.name ?? "the selected storage"} — machines joining this repository later will find it there`
                        : "Keep this project separate"}
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
              && repositoryFingerprintsRequireAcknowledgement(
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
          <Notice severity="danger" timing="blocking" className="sync-setup-system-error">
            {error}
          </Notice>
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

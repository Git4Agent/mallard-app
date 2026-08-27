import { homeDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";
import Icon from "../Icons";
import Notice from "../notice/Notice";
import { errorMessage } from "../../storageError";
import { storageCatalogApi } from "./api";
import { CATALOG_KINDS } from "./model";
import type {
  CatalogCapability,
  CatalogProviderKey,
  CatalogResourceKind,
  CatalogScopeView,
  LocalCatalogProfile,
} from "./types";

interface Props {
  workflowEpoch: number;
  isWorkflowCurrent: (workflowEpoch: number) => boolean;
  busy: boolean;
  onClose: () => void;
  onCreated: (scope: CatalogScopeView, workflowEpoch: number) => void;
}

function providerLabel(provider: CatalogProviderKey): string {
  return provider === "codex" ? "Codex" : "Claude";
}

// The two path helpers below are deliberately duplicated rather than imported
// from project-sync: this module may not depend on that one, and a dozen lines
// are the price of that boundary. Keep them in step with the originals in
// ProjectSetupWorkspace.tsx.
function expandHomePath(value: string, home: string | null): string {
  const trimmed = value.trim();
  if (!home || !trimmed.startsWith("~")) return trimmed;
  if (trimmed === "~") return home;
  if (trimmed.startsWith("~/")) {
    const separator = home.includes("\\") ? "\\" : "/";
    return `${home.replace(/[\\/]+$/, "")}${separator}${trimmed.slice(2)}`;
  }
  return trimmed;
}

function isAbsoluteProfilePath(value: string): boolean {
  return value.startsWith("/")
    || /^[A-Za-z]:[\\/]/.test(value)
    || value.startsWith("\\\\");
}

function attentionBadge(show: boolean) {
  if (!show) return null;
  return (
    <span className="sync-setup-state attention" aria-label="Needs review" title="Needs review">
      <Icon name="alert-triangle" size={14} />
    </span>
  );
}

/** Adding a Catalog row is Kind then provider home, and nothing else: no
 * storage, no repository, no resource. Those belong to the row's own page,
 * chosen per resource link. */
export default function AddCatalogScopeWorkspace({
  workflowEpoch,
  isWorkflowCurrent,
  busy,
  onClose,
  onCreated,
}: Props) {
  const [kind, setKind] = useState<CatalogResourceKind | null>(null);
  const [provider, setProvider] = useState<CatalogProviderKey | null>(null);
  const [profileId, setProfileId] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<LocalCatalogProfile[]>([]);
  const [capabilities, setCapabilities] = useState<CatalogCapability[]>([]);
  const [profileEditor, setProfileEditor] = useState<{
    provider: CatalogProviderKey;
    path: string;
  } | null>(null);
  const [homePath, setHomePath] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);
  const workflowIsActive = () => mountedRef.current && isWorkflowCurrent(workflowEpoch);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [nextProfiles, nextCapabilities] = await Promise.all([
          storageCatalogApi.listProfiles(),
          storageCatalogApi.listCapabilities(),
        ]);
        if (cancelled || !mountedRef.current) return;
        setProfiles(nextProfiles);
        setCapabilities(nextCapabilities);
      } catch (reason) {
        if (!cancelled && mountedRef.current) setError(errorMessage(reason));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void homeDir()
      .then((resolved) => {
        if (!cancelled) setHomePath(resolved.replace(/\/+$/, ""));
      })
      .catch(() => {
        if (!cancelled) setHomePath(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const supportedProviders = useMemo(
    () => capabilities
      .filter((capability) => kind === null || capability.kinds.includes(kind))
      .map((capability) => capability.provider),
    [capabilities, kind],
  );
  const compatibleProfiles = useMemo(
    () => profiles.filter((profile) => provider !== null && profile.provider_key === provider),
    [profiles, provider],
  );

  const chooseKind = (next: CatalogResourceKind) => {
    setKind(next);
    setError(null);
    // A provider the new kind cannot use is dropped rather than kept invalid.
    const stillSupported = capabilities
      .some((capability) => capability.kinds.includes(next)
        && capability.provider === provider);
    if (!stillSupported) {
      setProvider(null);
      setProfileId(null);
      setProfileEditor(null);
    }
  };

  const chooseProvider = (next: CatalogProviderKey | null) => {
    setProvider(next);
    setProfileEditor(null);
    setProfileId(
      next
        ? profiles.find((profile) => profile.provider_key === next)?.profile_id ?? null
        : null,
    );
  };

  const chooseProfileFolder = async () => {
    if (!provider) return;
    try {
      const picked = await open({ directory: true, multiple: false });
      if (!workflowIsActive() || typeof picked !== "string" || !picked) return;
      setProfileId(null);
      setProfileEditor({ provider, path: picked });
    } catch (reason) {
      if (workflowIsActive()) setError(errorMessage(reason));
    }
  };

  const working = busy || submitting;
  const expandedProfilePath = expandHomePath(profileEditor?.path ?? "", homePath);
  const customPathReady = profileEditor !== null && isAbsoluteProfilePath(expandedProfilePath);

  /** A pending path is saved as part of Add itself — its name derives from
   * the folder, and renaming afterwards lives on the row's page. If scope
   * creation then fails, the freshly saved profile stays selected so a retry
   * is just pressing Add again. */
  const submit = async () => {
    if (!kind || !provider) return;
    setSubmitting(true);
    setError(null);
    try {
      let chosenProfileId = profileId;
      if (chosenProfileId === null) {
        if (!customPathReady) return;
        const saved = await storageCatalogApi.saveProfile(provider, expandedProfilePath);
        if (!workflowIsActive()) return;
        setProfiles((current) => (
          current.some((profile) => profile.profile_id === saved.profile_id)
            ? current
            : [...current, saved]
        ));
        setProfileEditor(null);
        setProfileId(saved.profile_id);
        chosenProfileId = saved.profile_id;
      }
      const created = await storageCatalogApi.createScope(provider, chosenProfileId, kind);
      if (!workflowIsActive()) return;
      onCreated(created, workflowEpoch);
    } catch (reason) {
      if (!workflowIsActive()) return;
      setError(errorMessage(reason));
    } finally {
      if (workflowIsActive()) setSubmitting(false);
    }
  };

  const canAdd = kind !== null
    && provider !== null
    && (profileId !== null || customPathReady)
    && !working;
  const customPathHint = profileEditor
    ? !expandedProfilePath
      ? "Type or paste a path, or Browse. ~ means your home folder."
      : !isAbsoluteProfilePath(expandedProfilePath)
        ? "Paths must be absolute — start with ~/, / or a drive letter."
        : `${provider ? providerLabel(provider) : "Agent"} profile ·`
    : null;

  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section">
        <section
          className="sync-inline-new-project sync-setup-workspace"
          role="region"
          aria-labelledby="catalog-add-scope-title"
        >
          <header className="sync-modal-header">
            <div>
              <h1 id="catalog-add-scope-title">Add Catalog resource</h1>
            </div>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={onClose}
              disabled={submitting}
              aria-label="Close without adding a resource"
            >
              <Icon name="x" size={17} />
            </button>
          </header>

          <div className="sync-modal-body">
            {/* Kind */}
            <section className="sync-setup-section">
              <div className="sync-card-heading">
                <div><strong>Kind</strong></div>
                {attentionBadge(kind === null)}
              </div>
              <div className="sync-bundle-match-options" role="radiogroup" aria-label="Resource kind">
                {CATALOG_KINDS.map((option) => {
                  const active = kind === option.kind;
                  return (
                    <button
                      key={option.kind}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      className={`sync-bundle-match-option separate${active ? " active" : ""}`}
                      disabled={working}
                      onClick={() => chooseKind(option.kind)}
                    >
                      <span className="sync-bundle-radio"><span /></span>
                      <span className="sync-bundle-match-copy">
                        <strong>{option.label}</strong>
                        <span>{option.kind.startsWith("codex_pet") ? "Codex only" : "Codex and Claude"}</span>
                      </span>
                    </button>
                  );
                })}
              </div>
            </section>

            {/* Provider home */}
            <section className="sync-setup-section">
              <div className="sync-card-heading">
                <div><strong>Agent profile</strong></div>
                {attentionBadge(kind !== null
                  && (provider === null || (profileId === null && !customPathReady)))}
              </div>
              <div className="sync-profile-select-row">
                <span className="sync-select-shell">
                  <select
                    aria-label="Provider"
                    value={provider ?? ""}
                    disabled={working}
                    onChange={(event) => chooseProvider(
                      (event.target.value || null) as CatalogProviderKey | null,
                    )}
                  >
                    <option value="">Choose provider</option>
                    {supportedProviders.map((supported) => (
                      <option key={supported} value={supported}>{providerLabel(supported)}</option>
                    ))}
                  </select>
                </span>
              </div>
              {provider && (
                <div className={`sync-agent-profile-row${profileEditor ? " has-custom-path" : ""}`}>
                  <span className="sync-select-shell">
                    <select
                      aria-label={`${providerLabel(provider)} profile`}
                      value={profileEditor ? "__pending" : profileId ?? ""}
                      disabled={working}
                      aria-describedby={profileEditor ? `catalog-profile-path-hint-${provider}` : undefined}
                      onChange={(event) => {
                        const next = event.target.value;
                        if (next === "__pending") {
                          setProfileId(null);
                          setProfileEditor({ provider, path: "" });
                          return;
                        }
                        setProfileEditor(null);
                        setProfileId(next || null);
                      }}
                    >
                      <option value="">Choose profile</option>
                      {compatibleProfiles.map((profile) => (
                        <option key={profile.profile_id} value={profile.profile_id}>
                          {profile.display_name}
                        </option>
                      ))}
                      <option value="__pending">Custom path…</option>
                    </select>
                  </span>
                  {profileEditor && (
                    <input
                      value={profileEditor.path}
                      disabled={working}
                      aria-label={`${providerLabel(provider)} custom profile path`}
                      aria-describedby={`catalog-profile-path-hint-${provider}`}
                      placeholder={`${providerLabel(provider)} profile path`}
                      spellCheck={false}
                      autoFocus
                      onChange={(event) => setProfileEditor({
                        ...profileEditor,
                        path: event.target.value,
                      })}
                    />
                  )}
                  <button
                    type="button"
                    className="btn"
                    disabled={working}
                    onClick={() => void chooseProfileFolder()}
                  >
                    <Icon name="folder" size={13} /> Browse
                  </button>
                  {profileEditor && customPathHint && (
                    <p
                      id={`catalog-profile-path-hint-${provider}`}
                      className="sync-setup-path-hint mute"
                    >
                      {customPathHint}
                      {customPathReady && <code>{expandedProfilePath}</code>}
                    </p>
                  )}
                </div>
              )}
              {kind !== null && provider === null && (
                <small className="sync-setup-hint">
                  Each Catalog resource tracks one kind from one agent profile.
                </small>
              )}
            </section>

            {error && (
              /* The page's failure, not a field's: it wore the field-error
                 class for the colour and describes no control. Add is what
                 fills it — the profile save or the scope creation that did not
                 happen — so it announces assertively. The mount listing writes
                 here too, and a Catalog page that could not load its providers
                 is worth the same rung. */
              <Notice severity="danger" timing="blocking" className="sync-setup-system-error">
                {error}
              </Notice>
            )}
          </div>

          <footer className="sync-modal-footer sync-setup-footer">
            <div>
              <button
                type="button"
                className="btn btn-ghost"
                onClick={onClose}
                disabled={submitting}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn btn-primary"
                disabled={!canAdd}
                onClick={() => void submit()}
              >
                <Icon name="plus" size={13} />
                {submitting ? "Adding…" : "Add resource"}
              </button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

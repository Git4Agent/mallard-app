import { useEffect, useMemo, useRef, useState } from "react";
import Icon from "../Icons";
import { storageCatalogApi, type StorageCatalogApi } from "./api";
import { CATALOG_KINDS, errorMessage } from "./model";
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
  api?: StorageCatalogApi;
}

function providerLabel(provider: CatalogProviderKey): string {
  return provider === "codex" ? "Codex" : "Claude";
}

// Deliberately duplicated rather than imported from project-sync: this module
// may not depend on that one, and the six lines are the price of that
// boundary.
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
  api = storageCatalogApi,
}: Props) {
  const [kind, setKind] = useState<CatalogResourceKind | null>(null);
  const [provider, setProvider] = useState<CatalogProviderKey | null>(null);
  const [profileId, setProfileId] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<LocalCatalogProfile[]>([]);
  const [capabilities, setCapabilities] = useState<CatalogCapability[]>([]);
  const [profileEditor, setProfileEditor] = useState<{
    provider: CatalogProviderKey;
    displayName: string;
    path: string;
  } | null>(null);
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
          api.listProfiles(),
          api.listCapabilities(),
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
  }, [api]);

  const supportedProviders = useMemo(
    () => (kind
      ? capabilities
        .filter((capability) => capability.kinds.includes(kind))
        .map((capability) => capability.provider)
      : []),
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

  const saveProfile = async () => {
    if (!profileEditor) return;
    setError(null);
    try {
      const saved = await api.saveProfile(
        profileEditor.provider,
        profileEditor.displayName,
        profileEditor.path,
      );
      if (!workflowIsActive()) return;
      setProfiles((current) => [...current, saved]);
      setProfileEditor(null);
      setProfileId(saved.profile_id);
    } catch (reason) {
      if (workflowIsActive()) setError(errorMessage(reason));
    }
  };

  const removeProfile = async (profile: LocalCatalogProfile) => {
    setError(null);
    try {
      await api.deleteProfile(profile.profile_id, profile.revision);
      if (!workflowIsActive()) return;
      setProfiles((current) => current.filter((item) => item.profile_id !== profile.profile_id));
      if (profileId === profile.profile_id) setProfileId(null);
    } catch (reason) {
      if (workflowIsActive()) setError(errorMessage(reason));
    }
  };

  const submit = async () => {
    if (!kind || !provider || !profileId) return;
    setSubmitting(true);
    setError(null);
    try {
      const created = await api.createScope(provider, profileId, kind);
      if (!workflowIsActive()) return;
      onCreated(created, workflowEpoch);
    } catch (reason) {
      if (!workflowIsActive()) return;
      setError(errorMessage(reason));
    } finally {
      if (workflowIsActive()) setSubmitting(false);
    }
  };

  const working = busy || submitting;
  const canAdd = kind !== null && provider !== null && profileId !== null && !working;

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
              <h1 id="catalog-add-scope-title">Add Catalog row</h1>
            </div>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={onClose}
              disabled={submitting}
              aria-label="Close without adding a row"
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
                <div><strong>Provider home</strong></div>
                {attentionBadge(kind !== null && (provider === null || profileId === null))}
              </div>
              <div className="sync-profile-select-row">
                <span className="sync-select-shell">
                  <select
                    aria-label="Provider"
                    value={provider ?? ""}
                    disabled={working || !kind}
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
                <div className="sync-profile-select-row">
                  <span className="sync-select-shell">
                    <select
                      aria-label="Provider home"
                      value={profileId ?? ""}
                      disabled={working}
                      onChange={(event) => setProfileId(event.target.value || null)}
                    >
                      <option value="">Choose home</option>
                      {compatibleProfiles.map((profile) => (
                        <option key={profile.profile_id} value={profile.profile_id}>
                          {profile.display_name}
                        </option>
                      ))}
                    </select>
                  </span>
                  {!profileEditor && (
                    <button
                      type="button"
                      className="btn"
                      disabled={working}
                      onClick={() => setProfileEditor({ provider, displayName: "", path: "" })}
                    >
                      <Icon name="plus" size={13} /> Add provider home
                    </button>
                  )}
                </div>
              )}
              {profileEditor && (
                <form
                  className="catalog-inline-form"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void saveProfile();
                  }}
                >
                  <input
                    aria-label="Provider home name"
                    placeholder="Home name"
                    value={profileEditor.displayName}
                    onChange={(event) =>
                      setProfileEditor({ ...profileEditor, displayName: event.target.value })}
                  />
                  <input
                    aria-label="Provider home path"
                    placeholder="Absolute provider home path"
                    value={profileEditor.path}
                    onChange={(event) =>
                      setProfileEditor({ ...profileEditor, path: event.target.value })}
                  />
                  <div>
                    <button type="submit" className="btn btn-primary" disabled={working}>
                      Save home
                    </button>
                    <button type="button" className="btn btn-ghost" onClick={() => setProfileEditor(null)}>
                      Cancel
                    </button>
                  </div>
                </form>
              )}
              {provider && compatibleProfiles.length > 0 && (
                <details>
                  <summary>Manage saved homes</summary>
                  <div className="catalog-resource-list">
                    {compatibleProfiles.map((profile) => (
                      <div key={profile.profile_id} className="catalog-resource-row">
                        <div>
                          <strong>{profile.display_name}</strong>
                          <small>{profile.configured_home}</small>
                        </div>
                        <button
                          type="button"
                          className="btn btn-ghost danger"
                          disabled={working || profileId === profile.profile_id}
                          onClick={() => void removeProfile(profile)}
                        >
                          Remove
                        </button>
                      </div>
                    ))}
                  </div>
                </details>
              )}
              {kind !== null && provider === null && (
                <small className="sync-setup-hint">
                  Each row shows one kind from one provider home.
                </small>
              )}
            </section>

            {error && (
              <div className="sync-setup-field-error sync-setup-system-error" role="alert">
                <Icon name="alert-triangle" size={13} /> {error}
              </div>
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
                {submitting ? "Adding…" : "Add row"}
              </button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

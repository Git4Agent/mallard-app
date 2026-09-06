import { homeDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";
import Icon from "../Icons";
import Notice from "../notice/Notice";
import ProviderMark from "../sync-review/ProviderMark";
import { expandHomePath, isAbsoluteProfilePath } from "../../profilePath";
import { errorMessage } from "../../storageError";
import { storageCatalogApi } from "./api";
import { catalogDoors, catalogProviderList, type CatalogAddChoice } from "./model";
import type {
  CatalogCapabilityDescriptor,
  CatalogProfile,
  CatalogScopeDto,
} from "./types";

interface Props {
  workflowEpoch: number;
  isWorkflowCurrent: (workflowEpoch: number) => boolean;
  busy: boolean;
  /** The Kind a door chose on the way in, as a choice key; null asks. */
  initialChoiceKey: string | null;
  onClose: () => void;
  onCreated: (scope: CatalogScopeDto, workflowEpoch: number) => void;
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
 * chosen per resource link. Kind is a portable shape the backend registers,
 * offered in the backend's order; the providers offered are the ones it is
 * registered for. */
export default function AddCatalogScopeWorkspace({
  workflowEpoch,
  isWorkflowCurrent,
  busy,
  initialChoiceKey,
  onClose,
  onCreated,
}: Props) {
  // A key no registered shape answers to simply leaves Kind unchosen.
  const [choiceKey, setChoiceKey] = useState<string | null>(initialChoiceKey);
  const [provider, setProvider] = useState<string | null>(null);
  const [profileId, setProfileId] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<CatalogProfile[]>([]);
  const [capabilities, setCapabilities] = useState<CatalogCapabilityDescriptor[]>([]);
  const [profileEditor, setProfileEditor] = useState<{
    provider: string;
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

  /** The Add chooser's choices, each wearing the glyph of the shape it names:
   * the very list the Catalog landing draws as doors, so the door you opened
   * is still on screen, now as the choice it made. */
  const choices = useMemo(() => catalogDoors(capabilities), [capabilities]);
  const choice = choices.find((candidate) => candidate.key === choiceKey) ?? null;
  /** The providers on offer, with the backend's labels: those registered for
   * the chosen shape, or every registered provider before a shape is chosen,
   * so choosing a profile first stays usable. */
  const supportedProviders = useMemo(() => {
    const seen = new Map<string, string>();
    for (const capability of choice ? choice.providers : capabilities) {
      if (!seen.has(capability.provider)) seen.set(capability.provider, capability.provider_label);
    }
    return [...seen].map(([key, label]) => ({ key, label }));
  }, [capabilities, choice]);
  const labelForProvider = (key: string) =>
    capabilities.find((capability) => capability.provider === key)?.provider_label ?? key;
  const compatibleProfiles = useMemo(
    () => profiles.filter((profile) => provider !== null && profile.provider === provider),
    [profiles, provider],
  );
  const selectedProfile = compatibleProfiles.find((profile) => profile.profile_id === profileId) ?? null;

  const chooseKind = (next: CatalogAddChoice) => {
    setChoiceKey(next.key);
    setError(null);
    // A provider the new shape is not registered for is dropped rather than
    // kept invalid.
    const stillSupported = next.providers.some((capability) => capability.provider === provider);
    if (!stillSupported) {
      setProvider(null);
      setProfileId(null);
      setProfileEditor(null);
    }
  };

  const chooseProvider = (next: string | null) => {
    setProvider(next);
    setProfileEditor(null);
    setProfileId(
      next
        ? profiles.find((profile) => profile.provider === next)?.profile_id ?? null
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
  /** The one descriptor the two choices name: the chosen shape at the chosen
   * provider. Its coordinate is what the row records. */
  const descriptor = choice?.providers.find((capability) => capability.provider === provider) ?? null;

  /** A pending path is saved as part of Add itself — its name derives from
   * the folder, and renaming afterwards lives on the row's page. If scope
   * creation then fails, the freshly saved profile stays selected so a retry
   * is just pressing Add again. */
  const submit = async () => {
    if (!descriptor) return;
    setSubmitting(true);
    setError(null);
    try {
      let chosenProfileId = profileId;
      if (chosenProfileId === null) {
        if (!customPathReady) return;
        const saved = await storageCatalogApi.saveProfile(descriptor.provider, expandedProfilePath);
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
      const created = await storageCatalogApi.createScope(descriptor, chosenProfileId);
      if (!workflowIsActive()) return;
      onCreated(created, workflowEpoch);
    } catch (reason) {
      if (!workflowIsActive()) return;
      setError(errorMessage(reason));
    } finally {
      if (workflowIsActive()) setSubmitting(false);
    }
  };

  const canAdd = descriptor !== null
    && (profileId !== null || customPathReady)
    && !working;
  const customPathHint = profileEditor
    ? !expandedProfilePath
      ? "Type or paste a path, or Browse. ~ means your home folder."
      : !isAbsoluteProfilePath(expandedProfilePath)
        ? "Paths must be absolute — start with ~/, / or a drive letter."
        : `${provider ? labelForProvider(provider) : "Agent"} profile ·`
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
                {attentionBadge(choice === null)}
              </div>
              <div className="setup-choice-grid" role="radiogroup" aria-label="Resource kind">
                {choices.map((option) => {
                  const active = choiceKey === option.key;
                  return (
                    <button
                      key={option.key}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      className={`setup-choice-card${active ? " active" : ""}`}
                      disabled={working}
                      onClick={() => chooseKind(option)}
                    >
                      <span className="setup-choice-card-head">
                        <span className="setup-choice-mark" aria-hidden="true">
                          <Icon name={option.icon} size={24} />
                        </span>
                        <strong>{option.label}</strong>
                        <span className="setup-choice-card-check" aria-hidden="true">
                          {active && <Icon name="check" size={12} />}
                        </span>
                      </span>
                      <span className="setup-choice-card-foot">
                        {option.providers.map((capability) => (
                          <ProviderMark
                            key={capability.provider}
                            provider={capability.provider}
                            displayName={capability.provider_label}
                            size={12}
                          />
                        ))}
                        {catalogProviderList(option.providers.map((capability) => capability.provider_label))}
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
                {attentionBadge(choice !== null
                  && (provider === null || (profileId === null && !customPathReady)))}
              </div>
              {/* Two or three registered agents is a row, not a menu — and a
                  native select cannot hold the mark that tells them apart. */}
              <div className="setup-pill-row" role="radiogroup" aria-label="Provider">
                {supportedProviders.map((supported) => {
                  const active = provider === supported.key;
                  return (
                    <button
                      key={supported.key}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      className={`setup-pill${active ? " active" : ""}`}
                      disabled={working}
                      onClick={() => chooseProvider(supported.key)}
                    >
                      <ProviderMark provider={supported.key} displayName={supported.label} size={12} />
                      <span className="setup-pill-label">{supported.label}</span>
                      <span className="setup-pill-check" aria-hidden="true">
                        {active && <Icon name="check" size={12} />}
                      </span>
                    </button>
                  );
                })}
              </div>
              {provider && (
                /* The list is what supplies the row's container: without it
                   the narrow-window fallback, where the path field takes its
                   own line, has nothing to query and never fires. */
                <div className="sync-agent-profile-list">
                  <div className={`sync-agent-profile-row${profileEditor ? " has-custom-path" : ""}`}>
                    <span className="sync-select-shell">
                      <select
                        aria-label={`${labelForProvider(provider)} profile`}
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
                        aria-label={`${labelForProvider(provider)} custom profile path`}
                        aria-describedby={`catalog-profile-path-hint-${provider}`}
                        placeholder={`${labelForProvider(provider)} profile path`}
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
                    {/* A saved home says where it points in the same slot a
                        typed one does; column 2 holds Browse here, so this one
                        spans the row. */}
                    {!profileEditor && selectedProfile && (
                      <p className="sync-setup-path-hint ok spans-row">
                        {labelForProvider(provider)} profile ·<code>{selectedProfile.configured_home}</code>
                      </p>
                    )}
                  </div>
                </div>
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

import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState, type ReactNode } from "react";
import Icon from "../Icons";
import { FieldMessage } from "../notice/Notice";
import ProviderMark from "../sync-review/ProviderMark";
import {
  expandHomePath,
  isAbsoluteProfilePath,
  profilePathHint,
  type ProfilePathProbeState,
} from "../../profilePath";
import { errorMessage } from "../../storageError";
import { storageCatalogApi } from "./api";
import { unambiguousProfile } from "./model";
import { attentionBadge } from "./setupFurniture";
import type { CatalogCapabilityDescriptor, CatalogProfile, CatalogProfileProbe } from "./types";

/** One provider's answer: a saved profile, or a path being typed. */
type Selection =
  | { kind: "none" }
  | { kind: "existing"; profile: CatalogProfile }
  | { kind: "pending"; path: string };

interface Chosen {
  provider: string;
  selection: Selection;
}

interface Props {
  /** New project's two doors, drawn at the top of the body. */
  doors: ReactNode;
  /** The page's failure notice, drawn at the bottom of the body. */
  error: ReactNode;
  profiles: readonly CatalogProfile[];
  capabilities: readonly CatalogCapabilityDescriptor[];
  homePath: string | null;
  working: boolean;
  isActive: () => boolean;
  onError: (message: string) => void;
  onClose: () => void;
  /** Set up: the home as a saved profile — saved here when it was typed. */
  onChosen: (profile: CatalogProfile) => void;
}

/* Shorter than an autosave: the path is judged while you type, and the
   probe only reads the filesystem. */
const PATH_PROBE_DELAY_MS = 350;

/** The project's debounced path probe, against the Catalog's own read. */
function useProfilePathProbe(
  provider: string | null,
  path: string | null,
  isActive: () => boolean,
): ProfilePathProbeState<CatalogProfileProbe> | undefined {
  const [state, setState] = useState<ProfilePathProbeState<CatalogProfileProbe> | undefined>(undefined);
  const requestRef = useRef(0);
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  useEffect(() => {
    const requestId = ++requestRef.current;
    if (!provider || !path) {
      setState(undefined);
      return;
    }
    setState({ status: "checking" });
    const timer = window.setTimeout(() => {
      storageCatalogApi.probeProfile(provider, path).then(
        (probe) => {
          if (requestRef.current === requestId && isActiveRef.current()) setState({ status: "ok", probe });
        },
        (reason) => {
          if (requestRef.current === requestId && isActiveRef.current()) {
            setState({ status: "error", message: errorMessage(reason) });
          }
        },
      );
    }, PATH_PROBE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [provider, path]);
  return state;
}

/** The provider the page opens on, with New project's rule for its profile:
 *  the first provider with a saved profile — the backend lists profiles
 *  provider rank first — and that profile when it is the only one. */
function initialChoice(
  providers: readonly { key: string }[],
  profiles: readonly CatalogProfile[],
): Chosen | null {
  const provider = profiles[0]?.provider ?? providers[0]?.key ?? null;
  if (!provider) return null;
  const only = unambiguousProfile(profiles, provider);
  return { provider, selection: only ? { kind: "existing", profile: only } : { kind: "none" } };
}

/**
 * Step one of Add Catalog resource: the agent home, chosen the way New
 * project chooses its agents. One card per registered provider; the chosen
 * card opens the project's profile row — saved profiles and Custom path… in
 * one select, the inline path, Browse, the hint naming what the path
 * resolved to. One row per pass, so the toggle is a radio. Set up names the
 * home it opens, saving a typed path as a profile on the way.
 */
export default function CatalogHomeStep({
  doors,
  error,
  profiles,
  capabilities,
  homePath,
  working,
  isActive,
  onError,
  onClose,
  onChosen,
}: Props) {
  /* Every registered provider, in the backend's order, with its label. */
  const providers: { key: string; label: string }[] = [];
  for (const capability of capabilities) {
    if (!providers.some((candidate) => candidate.key === capability.provider)) {
      providers.push({ key: capability.provider, label: capability.provider_label });
    }
  }
  const [chosen, setChosen] = useState<Chosen | null>(null);
  const touched = useRef(false);
  /* The lists arrive after the page opens; until the user has acted, the
     opening choice follows them. */
  useEffect(() => {
    if (!touched.current) setChosen(initialChoice(providers, profiles));
  }, [profiles, capabilities]);
  const [submitting, setSubmitting] = useState(false);

  const provider = chosen?.provider ?? null;
  const label = providers.find((candidate) => candidate.key === provider)?.label ?? provider ?? "Agent";
  const selection: Selection = chosen?.selection ?? { kind: "none" };
  const pending = selection.kind === "pending";
  const expandedPath = pending ? expandHomePath(selection.path, homePath) : "";
  const probe = useProfilePathProbe(
    provider,
    pending && isAbsoluteProfilePath(expandedPath) ? expandedPath : null,
    isActive,
  );
  const hint = pending ? profilePathHint(probe, selection.path, homePath, { display_name: label }, profiles) : null;
  const resolvedProbe = pending && probe?.status === "ok" && hint?.tone === "ok" ? probe.probe : null;
  const existing = selection.kind === "existing" ? selection.profile : null;
  const ready = existing !== null || resolvedProbe !== null;
  const incomplete = selection.kind === "none" || (pending && !selection.path.trim());
  const busy = working || submitting;

  const choose = (next: Chosen) => {
    touched.current = true;
    setChosen(next);
  };
  const chooseProvider = (key: string) => {
    const only = unambiguousProfile(profiles, key);
    choose({ provider: key, selection: only ? { kind: "existing", profile: only } : { kind: "none" } });
  };
  const browse = async () => {
    if (!provider) return;
    try {
      const picked = await open({ directory: true, multiple: false });
      if (!isActive() || typeof picked !== "string" || !picked) return;
      choose({ provider, selection: { kind: "pending", path: picked } });
    } catch (reason) {
      if (isActive()) onError(errorMessage(reason));
    }
  };

  /* Set up saves a typed path as a profile first — named by its folder, as
     the backend names it; a home that is already a profile comes back as
     that profile — then opens the home's setup page. */
  const setUp = async () => {
    if (!provider || !ready || busy) return;
    if (existing) {
      onChosen(existing);
      return;
    }
    setSubmitting(true);
    try {
      const saved = await storageCatalogApi.saveProfile(provider, expandedPath);
      if (!isActive()) return;
      onChosen(saved);
    } catch (reason) {
      if (isActive()) onError(errorMessage(reason));
    } finally {
      if (isActive()) setSubmitting(false);
    }
  };

  const homeName = existing?.display_name ?? resolvedProbe?.suggested_name ?? null;
  const errorId = "catalog-setup-error-agent";
  const hintId = `catalog-profile-path-hint-${provider ?? "none"}`;

  return (
    <>
      <div className="sync-modal-body">
        {doors}
        <section className="sync-setup-section">
          <div className="sync-card-heading">
            <div><strong>Agent</strong></div>
            {attentionBadge(!ready)}
          </div>
          {/* The project's provider cards. One row per pass, so the toggle
              is a radio: the card you choose opens its profile row; the
              other is just its name. */}
          <div className="sync-provider-cards" role="radiogroup" aria-label="Agent home for this resource">
            {providers.map((candidate) => {
              const active = provider === candidate.key;
              const candidateProfiles = profiles.filter((profile) => profile.provider === candidate.key);
              return (
                <div key={candidate.key} className={`sync-provider-card${active ? " enabled" : ""}`}>
                  <label className="sync-provider-toggle setup-marked-toggle">
                    <input
                      type="radio"
                      name="catalog-home-provider"
                      checked={active}
                      disabled={busy}
                      aria-label={`Set up a ${candidate.label} home`}
                      onChange={() => chooseProvider(candidate.key)}
                    />
                    <ProviderMark provider={candidate.key} displayName={candidate.label} size={12} />
                    <strong>{candidate.label}</strong>
                  </label>
                  {active && (
                    <div className="sync-agent-profile-list">
                      <div className={`sync-agent-profile-row${pending ? " has-custom-path" : ""}${incomplete ? " has-error" : ""}`}>
                        <span className="sync-select-shell">
                          <select
                            value={existing ? existing.profile_id : pending ? "__pending" : ""}
                            disabled={busy}
                            aria-label={`${candidate.label} profile`}
                            aria-invalid={incomplete || undefined}
                            aria-describedby={incomplete ? errorId : undefined}
                            onChange={(event) => {
                              const next = event.target.value;
                              if (next === "__pending") choose({ provider: candidate.key, selection: { kind: "pending", path: "" } });
                              else if (!next) choose({ provider: candidate.key, selection: { kind: "none" } });
                              else {
                                const profile = candidateProfiles.find((item) => item.profile_id === next);
                                if (profile) choose({ provider: candidate.key, selection: { kind: "existing", profile } });
                              }
                            }}
                          >
                            <option value="">Choose profile</option>
                            {candidateProfiles.map((profile) => (
                              <option key={profile.profile_id} value={profile.profile_id}>{profile.display_name}</option>
                            ))}
                            <option value="__pending">Custom path…</option>
                          </select>
                        </span>
                        {pending && (
                          <input
                            value={selection.path}
                            disabled={busy}
                            aria-label={`${candidate.label} custom profile path`}
                            aria-invalid={hint?.tone === "bad" || undefined}
                            aria-describedby={[incomplete ? errorId : null, hintId].filter(Boolean).join(" ")}
                            placeholder={`${candidate.label} profile path`}
                            spellCheck={false}
                            autoFocus
                            onChange={(event) => choose({ provider: candidate.key, selection: { kind: "pending", path: event.target.value } })}
                          />
                        )}
                        <button type="button" className="btn" disabled={busy} onClick={() => void browse()}>
                          <Icon name="folder" size={13} /> Browse
                        </button>
                        {hint && (
                          /* Polite: it changes on a debounce while the field keeps focus. */
                          <p id={hintId} className={`sync-setup-path-hint ${hint.tone}`} aria-live="polite">
                            {hint.message}
                            {hint.path && <code>{hint.path}</code>}
                          </p>
                        )}
                        {existing && (
                          <p className="sync-setup-path-hint ok spans-row">
                            {candidate.label} profile ·<code>{existing.configured_home}</code>
                          </p>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
          {incomplete && provider && <FieldMessage id={errorId}>Choose a profile for {label}.</FieldMessage>}
        </section>
        {error}
      </div>
      <footer className="sync-modal-footer sync-setup-footer">
        <div>
          <button type="button" className="btn btn-ghost" onClick={onClose} disabled={submitting}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary" disabled={!ready || busy} onClick={() => void setUp()}>
            {submitting ? "Setting up…" : homeName ? `Set up ${homeName}` : "Set up"}
          </button>
        </div>
      </footer>
    </>
  );
}

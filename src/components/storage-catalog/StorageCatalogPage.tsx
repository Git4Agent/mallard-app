import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Icon from "../Icons";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import { storageCatalogApi, type StorageCatalogApi } from "./api";
import { actionLabel, catalogScopeLabel, errorMessage, linkStateLabel } from "./model";
import type {
  CatalogLinkInspection,
  CatalogProviderKey,
  CatalogResourcePullPlan,
  CatalogResourcePushPlan,
  CatalogResourceStatus,
  CatalogResourceView,
  CatalogScopeResources,
  CatalogScopeView,
} from "./types";

export interface CatalogStorageOption {
  id: string;
  name: string;
  kind: "local" | "s3";
}

type ReviewMode =
  | { kind: "browse" }
  | { kind: "push"; linkId: string; plan: CatalogResourcePushPlan }
  | { kind: "pull"; linkId: string; plan: CatalogResourcePullPlan };

function providerLabel(provider: CatalogProviderKey): string {
  return provider === "codex" ? "Codex" : "Claude";
}

function resourceKindLabel(resource: CatalogResourceView): string {
  if (resource.registration.resource_kind === "codex_pet_v1") return "Codex pet v1";
  if (resource.registration.resource_kind === "codex_pet_v2") return "Codex pet v2";
  if (resource.registration.resource_kind === "plugin_install_intent") return "Plugin intent";
  return resource.scope_label ? `${resource.scope_label} skill` : "Skill";
}

export default function StorageCatalogPage({
  scope,
  storages,
  onScopeGone,
  api = storageCatalogApi,
}: {
  scope: CatalogScopeView;
  storages: CatalogStorageOption[];
  /** The row disappeared underneath the page (deleted elsewhere). */
  onScopeGone: () => void;
  api?: StorageCatalogApi;
}) {
  const label = catalogScopeLabel(scope.scope, scope.profile);
  const [page, setPage] = useState<CatalogScopeResources | null>(null);
  const [browseStorageId, setBrowseStorageId] = useState<string | null>(null);
  const [activeRegistrationId, setActiveRegistrationId] = useState<string | null>(null);
  const [linkInspections, setLinkInspections] = useState<Record<string, CatalogLinkInspection>>({});
  const [mode, setMode] = useState<ReviewMode>({ kind: "browse" });
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<string | null>(null);
  const [forkName, setForkName] = useState("");
  const [pushIncluded, setPushIncluded] = useState(true);
  const [warningAcks, setWarningAcks] = useState<Set<string>>(new Set());
  const [pullActionId, setPullActionId] = useState<string | null>(null);
  const [destructiveAcknowledged, setDestructiveAcknowledged] = useState(false);
  const [copyName, setCopyName] = useState("");
  const [resourceStatus, setResourceStatus] = useState<CatalogResourceStatus | null>(null);
  // Per-row storage choice for the inline Link picker.
  const [rowStorage, setRowStorage] = useState<Record<string, string>>({});
  const epochRef = useRef(0);

  const load = useCallback(async () => {
    const epoch = ++epochRef.current;
    setLoading(true);
    setError(null);
    try {
      const next = await api.listScopeResources(scope.scope.scope_id, browseStorageId);
      if (epoch !== epochRef.current) return;
      setPage(next);
    } catch (reason) {
      if (epoch !== epochRef.current) return;
      if (errorMessage(reason).startsWith("unknown catalog scope")) {
        onScopeGone();
        return;
      }
      setError(errorMessage(reason));
    } finally {
      if (epoch === epochRef.current) setLoading(false);
    }
  }, [api, browseStorageId, onScopeGone, scope.scope.scope_id]);

  useEffect(() => {
    setMode({ kind: "browse" });
    void load();
    return () => {
      epochRef.current += 1;
    };
  }, [load]);

  useEffect(() => {
    setActiveRegistrationId(null);
    setOutcome(null);
    setBrowseStorageId(null);
  }, [scope.scope.scope_id]);

  const activeResource = useMemo(
    () => page?.registered.find((row) =>
      row.resource.registration.registration_id === activeRegistrationId)?.resource ?? null,
    [activeRegistrationId, page],
  );

  const inspectLink = useCallback(async (linkId: string, force: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const inspection = await api.inspectLink(linkId, force);
      setLinkInspections((current) => ({ ...current, [linkId]: inspection }));
      if (force) setOutcome("Resource refreshed from its provider home and linked storage.");
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }, [api]);

  useEffect(() => {
    if (!activeResource) return;
    for (const { link } of activeResource.links) {
      if (!linkInspections[link.link_id]) void inspectLink(link.link_id, false);
    }
  }, [activeResource, inspectLink, linkInspections]);

  const openPush = async (linkId: string) => {
    setBusy(true);
    setError(null);
    try {
      const plan = await api.preparePush(linkId);
      setPushIncluded(plan.row.selected_by_default);
      setWarningAcks(new Set());
      setMode({ kind: "push", linkId, plan });
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const publish = async (linkId: string, plan: CatalogResourcePushPlan) => {
    setBusy(true);
    setError(null);
    try {
      const result = await api.publish(linkId, plan.plan_id, pushIncluded, [...warningAcks]);
      setOutcome(result.message);
      setMode({ kind: "browse" });
      await inspectLink(linkId, false);
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const openPull = async (linkId: string, requestedCopyName?: string) => {
    setBusy(true);
    setError(null);
    try {
      const plan = await api.preparePull(linkId, requestedCopyName ?? null);
      setPullActionId(plan.item.default_action_id ?? null);
      setDestructiveAcknowledged(false);
      const preservation = plan.item.actions
        .map((action) => "copy" in action.kind ? action.kind.copy : null)
        .find((copy) => copy !== null);
      setCopyName(preservation?.copy_name ?? "");
      setMode({ kind: "pull", linkId, plan });
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const applyPull = async (linkId: string, plan: CatalogResourcePullPlan) => {
    setBusy(true);
    setError(null);
    try {
      const result = await api.applyPull(
        linkId,
        plan.plan_id,
        pullActionId,
        destructiveAcknowledged,
      );
      const detail = result.result.message
        ?? `${result.result.display_name}: ${result.result.outcome.replace(/_/g, " ")}`;
      setOutcome(result.remote_accepted
        ? detail
        : `Pull did not advance the comparison base. ${detail}`);
      setMode({ kind: "browse" });
      await inspectLink(linkId, false);
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const runFork = async (resource: CatalogResourceView) => {
    setBusy(true);
    setError(null);
    try {
      const plan = await api.prepareFork(resource.registration.registration_id, forkName.trim());
      const result = await api.applyFork(resource.registration.registration_id, plan.plan_id);
      setForkName("");
      setOutcome(result.message ?? `Created ${result.display_name}`);
      for (const { link } of resource.links) await inspectLink(link.link_id, false);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const unlink = async (resource: CatalogResourceView, linkId: string, revision: number) => {
    setBusy(true);
    setError(null);
    try {
      await api.unlink(resource.registration.registration_id, linkId, revision);
      setOutcome("Storage link removed. Local and remote content were left untouched.");
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const removeResource = async (resource: CatalogResourceView) => {
    setBusy(true);
    setError(null);
    try {
      await api.remove(
        resource.registration.registration_id,
        resource.registration.revision,
      );
      setActiveRegistrationId(null);
      setOutcome("Registration removed. Provider content was left untouched.");
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  /** Link never pushes or pulls; it changes configuration only. Linking a
   * stored resource routes straight into its Pull review. */
  const linkResource = async (
    selection: { localResourceId: string } | { repositoryId: string },
    storageId: string,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const result = await api.linkScopeResource({
        scope_id: scope.scope.scope_id,
        storage_id: storageId,
        local_resource_id: "localResourceId" in selection ? selection.localResourceId : null,
        repository_id: "repositoryId" in selection ? selection.repositoryId : null,
      });
      const storageName = storages.find((storage) => storage.id === storageId)?.name ?? storageId;
      setOutcome(`${result.resource.registration.display_name} linked to ${storageName} — configuration only, no provider content changed.`);
      await load();
      setActiveRegistrationId(result.resource.registration.registration_id);
      if (result.completion === "review_pull") await openPull(result.link_id);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (!activeResource) {
      setResourceStatus(null);
      return;
    }
    setResourceStatus(null);
    let cancelled = false;
    void api.getStatus(activeResource.registration.registration_id).then((status) => {
      if (!cancelled) setResourceStatus(status);
    }).catch((reason) => {
      if (!cancelled) setError(errorMessage(reason));
    });
    return () => {
      cancelled = true;
    };
  }, [activeResource, api]);

  if (activeResource) {
    const forkable = activeResource.registration.resource_kind !== "plugin_install_intent";
    const localPresent = page?.registered.find((row) =>
      row.resource.registration.registration_id === activeResource.registration.registration_id)
      ?.local_present ?? false;
    const linkedStorageIds = new Set(activeResource.links.map(({ link }) => link.storage_id));
    const linkableStorages = storages.filter((storage) => !linkedStorageIds.has(storage.id));
    const detailStorageId = rowStorage[activeResource.registration.registration_id]
      ?? linkableStorages[0]?.id ?? null;
    return (
      <main className="sync-main sync-project-links-page catalog-category-page">
        <section className="profile-links-section" aria-label={activeResource.registration.display_name}>
          <div className="profile-links-heading">
            <div className="profile-links-copy">
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy || mode.kind !== "browse"}
                onClick={() => {
                  setActiveRegistrationId(null);
                  setMode({ kind: "browse" });
                }}
              >
                <Icon name="chevron-left" size={13} /> {label}
              </button>
              <h1>{activeResource.registration.display_name}</h1>
              <p>{`${providerLabel(activeResource.registration.provider)} · ${resourceKindLabel(activeResource)}`}</p>
            </div>
            <div className="catalog-row-actions">
              {linkableStorages.length > 1 && (
                <span className="sync-select-shell">
                  <select
                    aria-label="Storage to link"
                    value={detailStorageId ?? ""}
                    disabled={busy || mode.kind !== "browse"}
                    onChange={(event) => setRowStorage((current) => ({
                      ...current,
                      [activeResource.registration.registration_id]: event.target.value,
                    }))}
                  >
                    {linkableStorages.map((storage) => (
                      <option key={storage.id} value={storage.id}>{storage.name}</option>
                    ))}
                  </select>
                </span>
              )}
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy || mode.kind !== "browse" || !localPresent || !detailStorageId}
                title={localPresent
                  ? "Link this resource to another storage"
                  : "The local resource is missing, so it cannot start a new stored resource"}
                onClick={() => detailStorageId && void linkResource(
                  { localResourceId: activeResource.registration.resource_id },
                  detailStorageId,
                )}
              >
                <Icon name="plus" size={13} /> Link storage
              </button>
            </div>
          </div>

          {error && <div className="sync-callout error" role="alert">{error}</div>}
          {outcome && (
            <div className="sync-callout success" role="status">
              <Icon name="check-circle" size={14} /> {outcome}
              <button type="button" className="btn btn-ghost" onClick={() => setOutcome(null)}>Dismiss</button>
            </div>
          )}
          {resourceStatus?.recovery_messages.map((message) => (
            <div key={message} className="sync-callout warning" role="status">{message}</div>
          ))}

          {mode.kind === "push" ? (
            <section className="catalog-review-card" aria-labelledby="catalog-push-title">
              <div className="catalog-review-heading">
                <div>
                  <h2 id="catalog-push-title">Push review</h2>
                  <p>{mode.plan.row.detail}</p>
                </div>
                <SyncReviewSelectionBar
                  count={pushIncluded ? 1 : 0}
                  total={1}
                  busy={busy}
                  recommendedTitle="Include the safe resource"
                  recommendedAriaLabel="Use recommended Push selection"
                  recommendedDisabled={pushIncluded || !mode.plan.row.selectable}
                  onRecommended={() => setPushIncluded(true)}
                  clearDisabled={!pushIncluded}
                  onClear={() => setPushIncluded(false)}
                  close={{
                    ariaLabel: "Close Push review",
                    onClose: () => setMode({ kind: "browse" }),
                  }}
                />
              </div>
              <label className="catalog-review-option">
                <input
                  type="checkbox"
                  checked={pushIncluded}
                  disabled={!mode.plan.row.selectable || busy}
                  onChange={(event) => setPushIncluded(event.target.checked)}
                />
                <span>{`Include ${mode.plan.row.display_name}`}</span>
              </label>
              {mode.plan.row.blocked_reason && (
                <div className="sync-callout warning">{mode.plan.row.blocked_reason}</div>
              )}
              {mode.plan.row.warnings.map((warning) => (
                <label key={warning.digest} className="catalog-review-option warning">
                  <input
                    type="checkbox"
                    checked={warningAcks.has(warning.digest)}
                    onChange={(event) => setWarningAcks((current) => {
                      const next = new Set(current);
                      if (event.target.checked) next.add(warning.digest);
                      else next.delete(warning.digest);
                      return next;
                    })}
                  />
                  <span>{warning.message}</span>
                </label>
              ))}
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy || !pushIncluded || mode.plan.row.warnings.some((warning) => !warningAcks.has(warning.digest))}
                onClick={() => void publish(mode.linkId, mode.plan)}
              >
                <Icon name="upload" size={13} /> Push resource
              </button>
            </section>
          ) : mode.kind === "pull" ? (
            <section className="catalog-review-card" aria-labelledby="catalog-pull-title">
              <div className="catalog-review-heading">
                <div>
                  <h2 id="catalog-pull-title">Pull review</h2>
                  <p>{mode.plan.item.detail ?? "Review the exact local action."}</p>
                </div>
                <SyncReviewSelectionBar
                  count={pullActionId ? 1 : 0}
                  total={mode.plan.item.actions.length > 0 ? 1 : 0}
                  busy={busy}
                  recommendedTitle="Use the recommended local action"
                  recommendedAriaLabel="Use recommended Pull selection"
                  recommendedDisabled={!mode.plan.item.default_action_id
                    || pullActionId === mode.plan.item.default_action_id}
                  onRecommended={() => {
                    setPullActionId(mode.plan.item.default_action_id ?? null);
                    setDestructiveAcknowledged(false);
                  }}
                  clearDisabled={pullActionId === null}
                  onClear={() => {
                    setPullActionId(null);
                    setDestructiveAcknowledged(false);
                  }}
                  close={{
                    ariaLabel: "Close Pull review",
                    onClose: () => setMode({ kind: "browse" }),
                  }}
                />
              </div>
              {mode.plan.conflicting_claims.length > 0 && (
                <div className="sync-callout warning">
                  A different version of this local target is claimed by {mode.plan.conflicting_claims
                    .map((claim) => storages.find((storage) => storage.id === claim.storage_id)?.name
                      ?? claim.storage_id)
                    .join(", ")}. Preserve a copy, acknowledge overwrite, or Skip.
                </div>
              )}
              {mode.plan.item.actions.length > 0 ? (
                <fieldset className="catalog-action-choices">
                  <legend>Local action</legend>
                  {mode.plan.item.actions.map((action) => (
                    <label key={action.action_id}>
                      <input
                        type="radio"
                        name="catalog-pull-action"
                        checked={pullActionId === action.action_id}
                        onChange={() => {
                          setPullActionId(action.action_id);
                          setDestructiveAcknowledged(false);
                        }}
                      />
                      <span>{actionLabel(action)}</span>
                    </label>
                  ))}
                  <label>
                    <input
                      type="radio"
                      name="catalog-pull-action"
                      checked={pullActionId === null}
                      onChange={() => {
                        setPullActionId(null);
                        setDestructiveAcknowledged(false);
                      }}
                    />
                    <span>Skip local change and accept the stored version as the comparison base</span>
                  </label>
                </fieldset>
              ) : (
                <div className="sync-callout info">No local mutation is required.</div>
              )}
              {copyName && (
                <form
                  className="catalog-inline-form"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void openPull(mode.linkId, copyName.trim());
                  }}
                >
                  <label>
                    <span>Preserved copy name</span>
                    <input value={copyName} onChange={(event) => setCopyName(event.target.value)} />
                  </label>
                  <button type="submit" className="btn btn-ghost" disabled={busy || copyName.trim() === ""}>
                    Update review
                  </button>
                </form>
              )}
              {mode.plan.item.actions.find((action) => action.action_id === pullActionId)?.requires_destructive_approval && (
                <label className="catalog-review-option warning">
                  <input
                    type="checkbox"
                    checked={destructiveAcknowledged}
                    onChange={(event) => setDestructiveAcknowledged(event.target.checked)}
                  />
                  <span>I understand this overwrites the current local resource.</span>
                </label>
              )}
              <button
                type="button"
                className="btn btn-primary"
                disabled={busy || Boolean(
                  mode.plan.item.actions.find((action) => action.action_id === pullActionId)
                    ?.requires_destructive_approval && !destructiveAcknowledged,
                )}
                onClick={() => void applyPull(mode.linkId, mode.plan)}
              >
                <Icon name="download" size={13} /> Pull resource
              </button>
            </section>
          ) : (
            <>
              {forkable && (
                <form
                  className="catalog-inline-form catalog-fork-form"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void runFork(activeResource);
                  }}
                >
                  <label>
                    <span>Fork local resource</span>
                    <input
                      placeholder={`${activeResource.registration.display_name}-local-copy`}
                      value={forkName}
                      onChange={(event) => setForkName(event.target.value)}
                    />
                  </label>
                  <button type="submit" className="btn btn-ghost" disabled={busy || forkName.trim() === ""}>
                    <Icon name="git-fork" size={13} /> Fork
                  </button>
                </form>
              )}
              <div className="catalog-link-grid">
                {activeResource.links.map(({ link, storage_name: storageName }) => {
                  const inspection = linkInspections[link.link_id];
                  const state = inspection?.state;
                  return (
                    <article key={link.link_id} className="catalog-link-card">
                      <div className="catalog-link-card-heading">
                        <div>
                          <h2>{storageName}</h2>
                          <span className={`catalog-state-badge ${state ?? "loading"}`}>
                            {state ? linkStateLabel(state) : "Checking…"}
                          </span>
                        </div>
                        <button
                          type="button"
                          className="sync-history-icon-action"
                          title="Refresh local and stored resource"
                          aria-label={`Refresh ${storageName}`}
                          disabled={busy}
                          onClick={() => void inspectLink(link.link_id, true)}
                        >
                          <Icon name="refresh" size={14} />
                        </button>
                      </div>
                      {inspection?.blocker && <div className="sync-callout error">{inspection.blocker}</div>}
                      <div className="catalog-link-meta">
                        <span>{`Repository ${link.repository_id.slice(0, 20)}…`}</span>
                        {inspection?.remote_revision != null && <span>{`Revision ${inspection.remote_revision}`}</span>}
                      </div>
                      <div className="catalog-link-actions">
                        <button
                          type="button"
                          className="btn btn-ghost"
                          disabled={busy || !inspection?.remote_present || state === "blocked"}
                          onClick={() => void openPull(link.link_id)}
                        >
                          <Icon name="download" size={13} /> Pull
                        </button>
                        <button
                          type="button"
                          className="btn btn-primary"
                          disabled={busy || !inspection?.local_present
                            || !["new", "local_changed"].includes(state ?? "")}
                          onClick={() => void openPush(link.link_id)}
                        >
                          <Icon name="upload" size={13} /> Push
                        </button>
                        <button
                          type="button"
                          className="btn btn-ghost danger"
                          disabled={busy}
                          onClick={() => void unlink(activeResource, link.link_id, link.revision)}
                        >
                          Unlink
                        </button>
                      </div>
                    </article>
                  );
                })}
                {activeResource.links.length === 0 && (
                  <div className="profile-links-empty">This resource has no storage links.</div>
                )}
              </div>
              {activeResource.links.length === 0 && (
                <button
                  type="button"
                  className="btn btn-ghost danger"
                  disabled={busy}
                  onClick={() => void removeResource(activeResource)}
                >
                  <Icon name="trash" size={13} /> Remove registration
                </button>
              )}
            </>
          )}
        </section>
      </main>
    );
  }

  return (
    <main className="sync-main sync-project-links-page catalog-category-page">
      <section className="profile-links-section" aria-label={label}>
        <div className="profile-links-heading">
          <div className="profile-links-copy">
            <h1>{label}</h1>
            <p>{`Reads ${scope.profile.configured_home}. Each resource links to storages independently.`}</p>
          </div>
        </div>
        {error && <div className="sync-callout error" role="alert">{error}</div>}
        {outcome && <div className="sync-callout success" role="status">{outcome}</div>}
        {loading || !page ? (
          <div className="profile-links-empty">Loading {label}…</div>
        ) : (
          <>
            <div className="catalog-resource-list">
              {page.registered.map((row) => (
                <article key={row.resource.registration.registration_id} className="catalog-resource-row">
                  <div>
                    <strong>{row.resource.registration.display_name}</strong>
                    <small>
                      {`${resourceKindLabel(row.resource)} · ${row.resource.links.length} link${row.resource.links.length === 1 ? "" : "s"}${row.local_present ? "" : " · missing locally"}`}
                    </small>
                  </div>
                  <button
                    type="button"
                    className="btn btn-ghost"
                    onClick={() => setActiveRegistrationId(row.resource.registration.registration_id)}
                  >
                    Open <Icon name="chevron-right" size={13} />
                  </button>
                </article>
              ))}
              {page.discovered.map((candidate) => {
                const storageId = rowStorage[candidate.resource_id] ?? storages[0]?.id ?? null;
                return (
                  <article key={candidate.resource_id} className="catalog-resource-row">
                    <div>
                      <strong>{candidate.display_name}</strong>
                      <small>
                        {candidate.scope_label
                          ? `${candidate.scope_label} · discovered, not linked`
                          : "Discovered, not linked"}
                      </small>
                    </div>
                    <div className="catalog-row-actions">
                      {storages.length > 1 && (
                        <span className="sync-select-shell">
                          <select
                            aria-label={`Storage for ${candidate.display_name}`}
                            value={storageId ?? ""}
                            disabled={busy}
                            onChange={(event) => setRowStorage((current) => ({
                              ...current,
                              [candidate.resource_id]: event.target.value,
                            }))}
                          >
                            {storages.map((storage) => (
                              <option key={storage.id} value={storage.id}>{storage.name}</option>
                            ))}
                          </select>
                        </span>
                      )}
                      <button
                        type="button"
                        className="btn btn-primary"
                        disabled={busy || !storageId}
                        title={storageId
                          ? "Link changes configuration only; Push shares content later"
                          : "Add a storage first"}
                        onClick={() => storageId && void linkResource(
                          { localResourceId: candidate.resource_id },
                          storageId,
                        )}
                      >
                        <Icon name="link" size={13} /> Link
                      </button>
                    </div>
                  </article>
                );
              })}
              {page.registered.length === 0 && page.discovered.length === 0 && (
                <div className="profile-links-empty">
                  <span>{`Nothing under ${scope.profile.configured_home} yet.`}</span>
                </div>
              )}
            </div>

            {storages.length > 0 && (
              <div className="sync-profile-select-row catalog-browse-storage">
                <span className="sync-select-shell">
                  <select
                    aria-label="Browse a storage for stored resources"
                    value={browseStorageId ?? ""}
                    disabled={busy}
                    onChange={(event) => setBrowseStorageId(event.target.value || null)}
                  >
                    <option value="">Browse stored resources…</option>
                    {storages.map((storage) => (
                      <option key={storage.id} value={storage.id}>{storage.name}</option>
                    ))}
                  </select>
                </span>
              </div>
            )}
            {browseStorageId && (
              <div className="catalog-resource-list">
                {page.remote_candidates.map((candidate) => (
                  <article key={candidate.repository_id} className="catalog-resource-row">
                    <div>
                      <strong>{candidate.display_name}</strong>
                      <small>
                        {`Stored revision ${candidate.head_revision}${candidate.scope_label ? ` · ${candidate.scope_label}` : ""}`}
                      </small>
                    </div>
                    <button
                      type="button"
                      className="btn btn-ghost"
                      disabled={busy}
                      title="Link this stored resource, then review what Pull would change"
                      onClick={() => void linkResource(
                        { repositoryId: candidate.repository_id },
                        browseStorageId,
                      )}
                    >
                      <Icon name="link" size={13} /> Link for review
                    </button>
                  </article>
                ))}
                {page.remote_candidates.length === 0 && (
                  <div className="profile-links-empty">
                    No unlinked stored resources of this kind on that storage.
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </section>
    </main>
  );
}

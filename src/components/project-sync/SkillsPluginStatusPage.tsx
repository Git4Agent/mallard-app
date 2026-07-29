import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  CapabilityStatusItem,
  CapabilityStatusReport,
  LocalProjectSummary,
  ProjectBinding,
} from "../../types";
import Icon from "../Icons";
import { projectSyncApi } from "./api";
import { errorMessage, providerLabel } from "./model";

export type CapabilityStatusView = "skills" | "plugins";

interface Props {
  view: CapabilityStatusView;
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  activeStorageId?: string | null;
  activeStorageName?: string | null;
  refreshEpoch: number;
  onOpenProjectSettings: () => void;
  selectionMode?: "push" | "pull";
  selectedResourceIds?: ReadonlySet<string>;
  selectableResourceIds?: ReadonlySet<string>;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
  reportOverride?: CapabilityStatusReport | null;
}

interface ContentProps {
  view: CapabilityStatusView;
  report: CapabilityStatusReport | null;
  loading: boolean;
  error: string | null;
  missingProfile: boolean;
  activeStorageName?: string | null;
  onRefresh: () => void;
  onOpenProjectSettings: () => void;
  selectionMode?: "push" | "pull";
  selectedResourceIds?: ReadonlySet<string>;
  selectableResourceIds?: ReadonlySet<string>;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
}

type StatusIcon =
  | "badge-check"
  | "cloud-off"
  | "download"
  | "git-compare-arrows"
  | "octagon-x"
  | "upload";

interface StatusPresentation {
  icon: StatusIcon;
  className: string;
  label: string;
  detail: string;
}

function shortDigest(value?: string | null): string {
  return value ? value.slice(0, 10) : "—";
}

function capabilityScope(item: CapabilityStatusItem): string {
  return item.kind === "plugin" ? "global plugin" : "global custom skill";
}

export function capabilityStatusPresentation(
  item: CapabilityStatusItem,
  storageName = "storage",
): StatusPresentation {
  if (item.blocked_reason || item.state === "blocked") {
    return {
      icon: "octagon-x",
      className: "blocked",
      label: "Blocked",
      detail: item.blocked_reason || "Mallard cannot safely capture this resource.",
    };
  }
  if (item.kind === "plugin" && item.enabled === false) {
    return {
      icon: "cloud-off",
      className: "disabled",
      label: "Disabled",
      detail: "Installed locally but disabled in the provider configuration.",
    };
  }
  switch (item.state) {
    case "synced":
      return item.kind === "plugin"
        ? {
          icon: "badge-check",
          className: "synced",
          label: "Backup intent matches",
          detail: `The local plugin installation intent matches ${storageName}; plugin payload bytes are installer-owned.`,
        }
        : {
          icon: "badge-check",
          className: "synced",
          label: "Up to date",
          detail: `The local skill content matches ${storageName}.`,
        };
    case "local_only":
      return {
        icon: "upload",
        className: "local",
        label: "Not backed up",
        detail: `This resource exists locally but not in ${storageName}.`,
      };
    case "local_ahead":
      return {
        icon: "upload",
        className: "local",
        label: "Local changes",
        detail: `This resource changed locally after the reviewed ${storageName} version.`,
      };
    case "storage_only":
      return {
        icon: "download",
        className: "storage",
        label: item.kind === "plugin" ? "Available to install" : "Available to pull",
        detail: `This resource is available in ${storageName} but not on this machine.`,
      };
    case "storage_ahead":
      return {
        icon: "download",
        className: "storage",
        label: "Storage update",
        detail: `${storageName} changed after the reviewed local version.`,
      };
    case "diverged":
      return {
        icon: "git-compare-arrows",
        className: "diverged",
        label: item.kind === "plugin" ? "Review intent" : "Review changes",
        detail: `The local and ${storageName} versions changed independently.`,
      };
    case "not_compared":
      return item.local_present
        ? {
          icon: "cloud-off",
          className: "unknown",
          label: item.kind === "plugin" ? "Installed" : "Available locally",
          detail: "Select a reachable storage to compare backup status.",
        }
        : {
          icon: "cloud-off",
          className: "unknown",
          label: "Unavailable",
          detail: "This resource is not available on this machine.",
        };
    default:
      return {
        icon: "cloud-off",
        className: "unknown",
        label: "Status unknown",
        detail: item.message || "Mallard could not safely compare this resource.",
      };
  }
}

function CapabilityRow({
  item,
  storageName,
  selectionMode,
  selected = false,
  selectable = false,
  selectionDisabled = false,
  onToggleResource,
}: {
  item: CapabilityStatusItem;
  storageName: string;
  selectionMode?: "push" | "pull";
  selected?: boolean;
  selectable?: boolean;
  selectionDisabled?: boolean;
  onToggleResource?: (resourceId: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const presentation = capabilityStatusPresentation(item, storageName);
  const blockedForPush = selectionMode === "push" && (!!item.blocked_reason || item.state === "blocked");
  const canSelect = selectable && !blockedForPush;
  const included = selected && canSelect;
  const detailsId = `capability-${item.resource_id.replace(/[^a-z0-9_-]/gi, "-")}`;
  const installDirectory = item.metadata?.install_dir_name;
  const marketplace = item.metadata?.plugin_marketplace;
  const source = item.metadata?.plugin_source;
  const providedSummary = item.provided_skills.length > 0
    ? item.provided_skills.join(" · ")
    : null;

  return (
    <article className={`sync-capability-row${expanded ? " expanded" : ""}${selectionMode && canSelect ? " sync-capability-selectable" : ""}${included ? " selected" : ""}`}>
      {selectionMode && canSelect && (
        <label
          className="sync-sync-row-choice sync-capability-choice"
          title={`${included ? "Exclude" : "Include"} ${item.display_name}`}
        >
          <input
            type="checkbox"
            checked={included}
            disabled={selectionDisabled}
            aria-label={`${included ? "Exclude" : "Include"} ${item.display_name}`}
            onChange={() => onToggleResource?.(item.resource_id)}
          />
        </label>
      )}
      <button
        type="button"
        className="sync-capability-row-main"
        onClick={() => setExpanded((current) => !current)}
        aria-expanded={expanded}
        aria-controls={detailsId}
      >
        <span
          className={`sync-capability-state ${presentation.className}`}
          title={presentation.detail}
          aria-label={presentation.detail}
        >
          <Icon name={presentation.icon} size={14} />
        </span>
        <span className="sync-capability-kind">
          <Icon name={item.kind === "plugin" ? "link" : "folder"} size={15} />
        </span>
        <span className="sync-capability-copy">
          <strong>{item.display_name}</strong>
          <span>
            {providerLabel(item.provider)} · {capabilityScope(item)}
            {installDirectory && item.kind === "standalone_skill" ? ` · folder ${installDirectory}` : ""}
          </span>
          {providedSummary && (
            <span className="sync-capability-provides" title={`Provides ${providedSummary}`}>
              Provides {providedSummary}
            </span>
          )}
        </span>
        <span className="sync-capability-row-status">
          <strong>{presentation.label}</strong>
          <span>{item.selected_in_recipe ? "In backup selection" : "Not selected for backup"}</span>
        </span>
        <Icon name={expanded ? "chevron-down" : "chevron-right"} size={13} className="sync-capability-chevron" />
      </button>

      {expanded && (
        <div id={detailsId} className="sync-capability-details">
          <p className={`sync-capability-detail-message ${presentation.className}`}>{item.message || presentation.detail}</p>
          <dl>
            <div>
              <dt>Local</dt>
              <dd>
                {item.local_present ? "Present" : "Missing"}
                {item.local_version ? ` · v${item.local_version}` : ""}
                {item.local_digest ? ` · ${item.kind === "plugin" ? "intent " : ""}${shortDigest(item.local_digest)}` : ""}
              </dd>
            </div>
            <div>
              <dt>{storageName}</dt>
              <dd>
                {item.storage_present ? "Present" : "Missing"}
                {item.storage_version ? ` · v${item.storage_version}` : ""}
                {item.storage_digest ? ` · ${item.kind === "plugin" ? "intent " : ""}${shortDigest(item.storage_digest)}` : ""}
              </dd>
            </div>
            <div><dt>Resource ID</dt><dd><code>{item.resource_id}</code></dd></div>
            <div><dt>Apply</dt><dd>{item.apply_policy.replace(/_/g, " ")}</dd></div>
            {marketplace && <div><dt>Marketplace</dt><dd>{marketplace}</dd></div>}
            {source && <div><dt>Source</dt><dd><code>{source}</code></dd></div>}
          </dl>
          {item.logical_paths.length > 0 && (
            <div className="sync-capability-paths">
              <span>Portable paths</span>
              <code>{item.logical_paths.join("\n")}</code>
            </div>
          )}
        </div>
      )}
    </article>
  );
}

export function SkillsPluginStatusContent({
  view,
  report,
  loading,
  error,
  missingProfile,
  activeStorageName,
  onRefresh,
  onOpenProjectSettings,
  selectionMode,
  selectedResourceIds = new Set(),
  selectableResourceIds,
  selectionDisabled = false,
  onToggleResource,
}: ContentProps) {
  const isSkillsView = view === "skills";
  const items = useMemo(
    () => report?.items.filter((item) => (
      isSkillsView ? item.kind !== "plugin" : item.kind === "plugin"
    )) ?? [],
    [isSkillsView, report?.items],
  );
  const storageName = report?.storage_name || activeStorageName || "storage";
  const title = isSkillsView ? "Skill status" : "Plugin status";
  const itemLabel = isSkillsView ? "skills" : "plugins";
  const icon = isSkillsView ? "folder" : "link";
  const headingId = `project-${view}-heading`;
  const compactSelectionReview = selectionMode != null;

  return (
    <div
      className={`sync-capability-page sync-history-embedded${compactSelectionReview ? " sync-history-selection-review" : ""}`}
      aria-labelledby={compactSelectionReview ? undefined : headingId}
      aria-label={compactSelectionReview ? title : undefined}
    >
      {!compactSelectionReview && <header className="profile-links-heading sync-history-header">
        <div className="profile-links-copy">
          <h2 id={headingId} className="sync-history-embedded-title"><Icon name={icon} size={15} />{title}</h2>
        </div>
        <div className="sync-history-toolbar">
          {(report?.storage_name || activeStorageName) && (
            <span className="sync-history-storage-lens" title={`Comparing with ${storageName}`}>
              <Icon name="link" size={12} /><span>{storageName}</span>
            </span>
          )}
          {report && (
            <span className="sync-history-toolbar-stats">
              <span className="sync-history-heading-count sync-history-toolbar-count" title={`${items.length} ${itemLabel}`} aria-label={`${items.length} ${itemLabel}`}>
                <Icon name={icon} size={12} />{items.length}
              </span>
            </span>
          )}
          {report && report.warnings.length > 0 && (
            <details className="sync-capability-warnings">
              <summary><Icon name="alert-triangle" size={13} />{report.warnings.length} warning{report.warnings.length === 1 ? "" : "s"}</summary>
              <ul>{report.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>
            </details>
          )}
          <button
            type="button"
            className="sync-history-icon-action"
            onClick={onRefresh}
            disabled={loading || missingProfile}
            title={`Refresh ${itemLabel}`}
            aria-label={`Refresh ${itemLabel}`}
          >
            <Icon name="refresh" size={15} className={loading ? "icon-spin" : undefined} />
          </button>
        </div>
      </header>}

      {missingProfile ? (
        <div className="sync-history-state sync-history-profile-state">
          <Icon name="alert-triangle" size={18} />
          <div><strong>Choose an agent profile to inventory {itemLabel}.</strong><span>The status view reads only the profile assigned to this project.</span></div>
          <button type="button" className="btn btn-primary" onClick={onOpenProjectSettings}>Open Project Settings</button>
        </div>
      ) : loading && !report ? (
        <div className="sync-history-state" role="status" aria-live="polite"><span className="status-loader" /> Scanning {itemLabel}…</div>
      ) : error && !report ? (
        <div className="sync-history-state sync-capability-error" role="alert">
          <Icon name="alert-triangle" size={18} /><span>{error}</span><button type="button" className="btn" onClick={onRefresh}>Retry</button>
        </div>
      ) : report ? (
        <>
          {error && <div className="sync-callout error sync-capability-inline-error" role="alert"><Icon name="alert-triangle" size={14} />{error}</div>}
          {items.length === 0 ? (
            <div className="sync-history-state"><Icon name={icon} size={20} /> No {itemLabel} were found for this project and profile.</div>
          ) : (
            <div className="sync-capability-list sync-capability-primary-list">
              {items.map((item) => <CapabilityRow key={item.resource_id} item={item} storageName={storageName}
                selectionMode={selectionMode} selected={selectedResourceIds.has(item.resource_id)}
                selectable={selectableResourceIds?.has(item.resource_id) ?? !!selectionMode}
                selectionDisabled={selectionDisabled} onToggleResource={onToggleResource} />)}
            </div>
          )}
        </>
      ) : null}
    </div>
  );
}

export default function SkillsPluginStatusPage({
  view,
  project,
  binding,
  activeStorageId,
  activeStorageName,
  refreshEpoch,
  onOpenProjectSettings,
  selectionMode,
  selectedResourceIds,
  selectableResourceIds,
  selectionDisabled = false,
  onToggleResource,
  reportOverride,
}: Props) {
  const [report, setReport] = useState<CapabilityStatusReport | null>(reportOverride ?? null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestRef = useRef(0);
  const missingProfile = !binding || Object.keys(binding.profile_ids ?? {}).length === 0;

  const load = useCallback(async () => {
    if (reportOverride !== undefined) {
      requestRef.current += 1;
      setReport(reportOverride);
      setError(null);
      setLoading(false);
      return;
    }
    if (missingProfile) {
      setReport(null);
      setError(null);
      setLoading(false);
      return;
    }
    const requestId = ++requestRef.current;
    setLoading(true);
    setError(null);
    try {
      const next = await projectSyncApi.getCapabilityStatus(project.local_project_id, activeStorageId);
      if (requestRef.current === requestId) setReport(next);
    } catch (reason) {
      if (requestRef.current === requestId) setError(errorMessage(reason));
    } finally {
      if (requestRef.current === requestId) setLoading(false);
    }
  }, [activeStorageId, missingProfile, project.local_project_id, reportOverride]);

  useEffect(() => {
    setReport(null);
    void load();
    return () => {
      requestRef.current += 1;
    };
  }, [load, refreshEpoch]);

  return (
    <SkillsPluginStatusContent
      view={view}
      report={report}
      loading={loading}
      error={error}
      missingProfile={missingProfile}
      activeStorageName={activeStorageName}
      onRefresh={() => void load()}
      onOpenProjectSettings={onOpenProjectSettings}
      selectionMode={selectionMode}
      selectedResourceIds={selectedResourceIds}
      selectableResourceIds={selectableResourceIds}
      selectionDisabled={selectionDisabled}
      onToggleResource={onToggleResource}
    />
  );
}

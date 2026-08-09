import { useMemo, useState } from "react";
import type { ProjectResourceCategory, ProjectResourceDescriptor } from "../../types";
import Icon from "../Icons";
import { RESOURCE_GROUPS, categoryFor, providerLabel } from "./model";

interface Props {
  resources: ProjectResourceDescriptor[];
  selected: Set<string>;
  statuses: Map<string, string>;
  selectableResourceIds?: ReadonlySet<string>;
  disabled?: boolean;
  onToggle: (resourceId: string) => void;
}

const CATEGORY_ICON: Record<ProjectResourceCategory, "activity" | "settings" | "folder"> = {
  conversations: "activity",
  project_files: "folder",
  tools: "settings",
};

function statusLabel(status?: string): string {
  if (!status) return "Not compared";
  return status.replace(/_/g, " ");
}

function resourceIcon(resource: ProjectResourceDescriptor): "activity" | "file" | "settings" {
  if (resource.kind === "requirement") return "settings";
  if (resource.kind.includes("conversation")) return "activity";
  return "file";
}

function ResourceRow({
  resource,
  checked,
  status,
  selectable,
  disabled,
  onToggle,
}: {
  resource: ProjectResourceDescriptor;
  checked: boolean;
  status?: string;
  selectable: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const blocked = !!resource.blocked_reason;

  return (
    <div className={`sync-resource-row${selectable ? " sync-resource-row-selectable" : ""}${checked ? " selected" : ""}${blocked ? " blocked" : ""}${expanded ? " expanded" : ""}`}>
      {selectable && (
        <label className="sync-resource-select">
          <input
            type="checkbox"
            checked={checked}
            disabled={disabled}
            onChange={onToggle}
            aria-label={`Include ${resource.display_name}`}
          />
        </label>
      )}
      <button
        type="button"
        className="sync-resource-main"
        onClick={() => setExpanded((current) => !current)}
        aria-expanded={expanded}
      >
        <span className="sync-resource-toggle">
          <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
        </span>
        <span className="sync-resource-kind-icon">
          <Icon name={resourceIcon(resource)} size={14} />
        </span>
        <span className="sync-resource-copy">
          <strong>{resource.display_name}</strong>
          <span>
            {providerLabel(resource.provider)} · {(resource.provider_resource_type ?? resource.kind).replace(/_/g, " ")}
          </span>
        </span>
        <span className="sync-resource-meta">
          {status && <span className={`sync-resource-status status-${status}`}>{statusLabel(status)}</span>}
        </span>
      </button>
      {expanded && (
        <div className="sync-resource-detail">
          {resource.description && <p>{resource.description}</p>}
          {resource.logical_paths && resource.logical_paths.length > 0 && (
            <div>
              <span>Portable paths</span>
              <code>{resource.logical_paths.join("\n")}</code>
            </div>
          )}
          <div className="sync-resource-detail-grid">
            <span>Resource ID</span><code>{resource.resource_id}</code>
            <span>Kind</span><code>{resource.kind}</code>
            {resource.apply_policy && <><span>Apply</span><code>{resource.apply_policy}</code></>}
          </div>
          {resource.blocked_reason && (
            <div className="sync-resource-blocker"><Icon name="alert-triangle" size={14} /> {resource.blocked_reason}</div>
          )}
        </div>
      )}
    </div>
  );
}

function ResourceGroup({
  group,
  items,
  selected,
  statuses,
  selectableResourceIds,
  disabled,
  onToggle,
}: {
  group: (typeof RESOURCE_GROUPS)[number];
  items: ProjectResourceDescriptor[];
  selected: Set<string>;
  statuses: Map<string, string>;
  selectableResourceIds?: ReadonlySet<string>;
  disabled: boolean;
  onToggle: (resourceId: string) => void;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const selectableItems = items.filter((item) => (
    !item.blocked_reason
    && item.apply_policy !== "never"
    && (selectableResourceIds?.has(item.resource_id) ?? true)
  ));
  const selectableIds = new Set(selectableItems.map((item) => item.resource_id));
  const selectedCount = selectableItems.filter((item) => selected.has(item.resource_id)).length;
  const allSelected = selectableItems.length > 0 && selectedCount === selectableItems.length;

  const toggleCollapsed = () => setCollapsed((current) => !current);

  return (
    <section className={`sync-resource-group category-${group.id}`}>
      <div
        className="sync-resource-group-header"
        role="button"
        tabIndex={0}
        aria-expanded={!collapsed}
        title={group.description}
        onClick={toggleCollapsed}
        onKeyDown={(event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          toggleCollapsed();
        }}
      >
        <span className="sync-resource-group-icon"><Icon name={CATEGORY_ICON[group.id]} size={16} /></span>
        <span className="sync-resource-group-copy">
          <strong>{group.label}</strong>
          <span>{group.description}</span>
        </span>
        <span className="sync-resource-group-count">{selectedCount}/{items.length}</span>
        <span className="sync-resource-group-actions" onClick={(event) => event.stopPropagation()}>
          <button
            type="button"
            className="btn-link sync-resource-group-all"
            disabled={disabled || selectableItems.length === 0}
            onClick={() => {
              for (const item of selectableItems) {
                if (selected.has(item.resource_id) === allSelected) onToggle(item.resource_id);
              }
            }}
          >
            {allSelected ? "Clear" : "Include all"}
          </button>
        </span>
        <Icon name={collapsed ? "chevron-right" : "chevron-down"} size={13} className="sync-resource-group-chevron" />
      </div>
      {!collapsed && (
        <div className="sync-resource-list">
          {items.map((resource) => (
            <ResourceRow
              key={resource.resource_id}
              resource={resource}
              checked={selectableIds.has(resource.resource_id) && selected.has(resource.resource_id)}
              status={statuses.get(resource.resource_id)}
              selectable={selectableIds.has(resource.resource_id)}
              disabled={disabled}
              onToggle={() => onToggle(resource.resource_id)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

export default function ResourceInventory({
  resources,
  selected,
  statuses,
  selectableResourceIds,
  disabled = false,
  onToggle,
}: Props) {
  const grouped = useMemo(() => {
    const result = new Map<ProjectResourceCategory, ProjectResourceDescriptor[]>();
    for (const group of RESOURCE_GROUPS) result.set(group.id, []);
    for (const resource of resources) result.get(categoryFor(resource))?.push(resource);
    return result;
  }, [resources]);

  if (resources.length === 0) {
    return (
      <div className="sync-empty-state compact">
        <Icon name="folder" size={22} />
        <strong>No project resources discovered</strong>
        <span>Refresh after the provider has created tasks, sessions, or project configuration.</span>
      </div>
    );
  }

  return (
    <div className="sync-resource-groups">
      {RESOURCE_GROUPS.map((group) => {
        const items = grouped.get(group.id) ?? [];
        if (items.length === 0) return null;
        return (
          <ResourceGroup
            key={group.id}
            group={group}
            items={items}
            selected={selected}
            statuses={statuses}
            selectableResourceIds={selectableResourceIds}
            disabled={disabled}
            onToggle={onToggle}
          />
        );
      })}
    </div>
  );
}

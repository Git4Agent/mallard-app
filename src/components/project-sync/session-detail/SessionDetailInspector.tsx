import { useEffect, useId, useRef, useState } from "react";
import type {
  ProjectSessionDetail,
  SessionCheckpointJump,
  SessionCount,
  SessionToolCategory,
} from "../../../types";
import Icon from "../../Icons";
import {
  formatSessionCount,
  SESSION_TOOL_CATEGORIES,
  SESSION_TOOL_CATEGORY_LABELS,
  sessionCountQualityLabel,
  sessionCountSupported,
  sessionToolCategoryCount,
  sessionToolParentState,
  toggleSessionToolParent,
  type SessionDetailFilters,
} from "./sessionDetailModel";

interface Props {
  detail: ProjectSessionDetail;
  filters: SessionDetailFilters;
  loadedToolCount: number;
  allLoadedToolsExpanded: boolean;
  selectedCheckpointId: string;
  onFiltersChange: (filters: SessionDetailFilters) => void;
  onCheckpointJump: (checkpoint: SessionCheckpointJump) => void;
  onToggleAllLoadedTools: () => void;
}

function CountLabel({ count }: { count: SessionCount }) {
  const quality = sessionCountQualityLabel(count);
  return (
    <span
      className={`session-detail-filter-count quality-${count.quality}`}
      title={count.explanation ?? quality ?? undefined}
      aria-label={count.explanation ?? (quality ? `${quality}: ${formatSessionCount(count)}` : undefined)}
    >
      {formatSessionCount(count)}
      {quality && count.quality === "inferred" && <span aria-hidden="true">~</span>}
    </span>
  );
}

function FilterCheckbox({
  label,
  checked,
  disabled,
  count,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  count: SessionCount;
  onChange: () => void;
}) {
  return (
    <label className={`session-detail-filter-row${disabled ? " disabled" : ""}`}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={onChange} />
      <span>{label}</span>
      <CountLabel count={count} />
    </label>
  );
}

function ToolParentCheckbox({
  detail,
  filters,
  onChange,
}: {
  detail: ProjectSessionDetail;
  filters: SessionDetailFilters;
  onChange: () => void;
}) {
  const state = sessionToolParentState(filters, detail.event_counts);
  const checkboxRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (checkboxRef.current) checkboxRef.current.indeterminate = state.indeterminate;
  }, [state.indeterminate]);
  return (
    <label className={`session-detail-filter-row tool-parent${state.disabled ? " disabled" : ""}`}>
      <input
        ref={checkboxRef}
        type="checkbox"
        checked={state.checked}
        disabled={state.disabled}
        aria-checked={state.indeterminate ? "mixed" : state.checked}
        onChange={onChange}
      />
      <span>Tool calls</span>
      <CountLabel count={detail.event_counts.tool_calls} />
    </label>
  );
}

export default function SessionDetailInspector({
  detail,
  filters,
  loadedToolCount,
  allLoadedToolsExpanded,
  selectedCheckpointId,
  onFiltersChange,
  onCheckpointJump,
  onToggleAllLoadedTools,
}: Props) {
  const [optionsOpen, setOptionsOpen] = useState(false);
  const optionsId = `session-view-options-${useId().replace(/:/g, "")}`;
  const counts = detail.event_counts;

  const toggleToolCategory = (category: SessionToolCategory) => {
    onFiltersChange({
      ...filters,
      toolCategories: {
        ...filters.toolCategories,
        [category]: !filters.toolCategories[category],
      },
    });
  };

  return (
    <aside className={`session-detail-inspector${optionsOpen ? " options-open" : ""}`} aria-label="Session view options">
      <button
        type="button"
        className="btn session-detail-options-toggle"
        aria-expanded={optionsOpen}
        aria-controls={optionsId}
        onClick={() => setOptionsOpen((current) => !current)}
      >
        <Icon name="settings" size={14} />
        View options
        <Icon name={optionsOpen ? "chevron-up" : "chevron-down"} size={12} />
      </button>

      <div id={optionsId} className="session-detail-inspector-body">
        <section className="session-detail-checkpoint-jump" aria-labelledby="session-checkpoint-jump-heading">
          <h2 id="session-checkpoint-jump-heading">Checkpoints</h2>
          <label>
            <span>Jump to</span>
            <select
              value={selectedCheckpointId}
              disabled={!sessionCountSupported(counts.checkpoints) || detail.checkpoint_jumps.length === 0}
              onChange={(changeEvent) => {
                const checkpoint = detail.checkpoint_jumps.find((candidate) => (
                  candidate.event_id === changeEvent.target.value
                ));
                if (checkpoint) onCheckpointJump(checkpoint);
              }}
            >
              <option value="">{detail.checkpoint_jumps.length ? "Choose a checkpoint" : "No checkpoints"}</option>
              {detail.checkpoint_jumps.map((checkpoint) => (
                <option key={checkpoint.event_id} value={checkpoint.event_id}>
                  {checkpoint.checkpoint.short_sha} · {checkpoint.checkpoint.subject}
                </option>
              ))}
            </select>
          </label>
          {counts.checkpoints.quality === "unsupported" && (
            <p>{counts.checkpoints.explanation ?? "Git checkpoints are unavailable for this project."}</p>
          )}
        </section>

        <fieldset className="session-detail-filters">
          <legend>Filters</legend>
          <FilterCheckbox
            label="Prompts"
            checked={filters.prompts}
            disabled={!sessionCountSupported(counts.prompts)}
            count={counts.prompts}
            onChange={() => onFiltersChange({ ...filters, prompts: !filters.prompts })}
          />
          <FilterCheckbox
            label="Responses"
            checked={filters.responses}
            disabled={!sessionCountSupported(counts.responses)}
            count={counts.responses}
            onChange={() => onFiltersChange({ ...filters, responses: !filters.responses })}
          />
          <FilterCheckbox
            label="Intermediate steps"
            checked={filters.intermediateSteps}
            disabled={!sessionCountSupported(counts.intermediate_steps)}
            count={counts.intermediate_steps}
            onChange={() => onFiltersChange({ ...filters, intermediateSteps: !filters.intermediateSteps })}
          />
          <FilterCheckbox
            label="Checkpoints"
            checked={filters.checkpoints}
            disabled={!sessionCountSupported(counts.checkpoints)}
            count={counts.checkpoints}
            onChange={() => onFiltersChange({ ...filters, checkpoints: !filters.checkpoints })}
          />

          <div className="session-detail-tool-filters">
            <ToolParentCheckbox
              detail={detail}
              filters={filters}
              onChange={() => onFiltersChange(toggleSessionToolParent(filters, counts))}
            />
            <div className="session-detail-tool-filter-children" aria-label="Tool call categories">
              {SESSION_TOOL_CATEGORIES.map((category) => {
                const count = sessionToolCategoryCount(counts, category);
                return (
                  <FilterCheckbox
                    key={category}
                    label={SESSION_TOOL_CATEGORY_LABELS[category]}
                    checked={filters.toolCategories[category]}
                    disabled={!filters.toolCalls || !sessionCountSupported(count)}
                    count={count}
                    onChange={() => toggleToolCategory(category)}
                  />
                );
              })}
            </div>
          </div>
        </fieldset>

        <section className="session-detail-tool-view" aria-labelledby="session-tool-view-heading">
          <h2 id="session-tool-view-heading">View</h2>
          <button
            type="button"
            className="btn btn-ghost"
            disabled={!filters.toolCalls || loadedToolCount === 0}
            onClick={onToggleAllLoadedTools}
          >
            <Icon name={allLoadedToolsExpanded ? "chevron-up" : "chevron-down"} size={12} />
            {allLoadedToolsExpanded ? "Collapse loaded tool calls" : "Expand loaded tool calls"}
          </button>
          <p>{loadedToolCount} individual tool {loadedToolCount === 1 ? "row" : "rows"} loaded</p>
        </section>
      </div>
    </aside>
  );
}

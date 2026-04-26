import type { CSSProperties } from "react";
import type { WorkItem } from "../../api.js";
import { miniButtonStyle } from "./plan-utils.js";

/**
 * Selection-aware action bar that hovers above the work-item list whenever
 * one or more rows are marked. Used by `PlanPane`. The actions mirror the
 * batch options that previously lived only behind the right-click menu on
 * a marked row, so a keyboard- or kebab-first user can reach them without
 * the right-click reflex.
 *
 * Pure helpers (`shouldShowSelectionActionBar`, `summarizeSelection`) are
 * exported so tests can exercise the rules without a DOM. The component
 * itself is a presentational wrapper — `PlanPane` owns the marked-set
 * state and provides callbacks. There is no separate store.
 */

export function shouldShowSelectionActionBar(markedCount: number): boolean {
  return markedCount >= 1;
}

export function summarizeSelection(markedCount: number): string {
  return markedCount === 1 ? "1 selected" : `${markedCount} selected`;
}

export interface SelectionActionBarProps {
  /** Marked work items (the ones the bar's actions apply to). */
  items: WorkItem[];
  onClear(): void;
  onChangeStatus(): void;
  onChangePriority(): void;
  onAddAllToAgent(): void;
  onDelete(): void;
}

export function SelectionActionBar({
  items,
  onClear,
  onChangeStatus,
  onChangePriority,
  onAddAllToAgent,
  onDelete,
}: SelectionActionBarProps) {
  if (!shouldShowSelectionActionBar(items.length)) return null;
  const lockedCount = items.filter((item) => item.status === "in_progress").length;
  const allLocked = lockedCount === items.length;
  return (
    <div
      data-testid="selection-action-bar"
      style={containerStyle}
    >
      <span data-testid="selection-action-bar-summary" style={summaryStyle}>
        {summarizeSelection(items.length)}
      </span>
      <button
        type="button"
        data-testid="selection-action-bar-clear"
        onClick={onClear}
        style={miniButtonStyle}
        title="Clear selection"
      >
        Clear
      </button>
      <span style={{ flex: 1 }} />
      <button
        type="button"
        data-testid="selection-action-bar-status"
        onClick={onChangeStatus}
        disabled={allLocked}
        style={{ ...miniButtonStyle, opacity: allLocked ? 0.4 : 1 }}
        title="Change status for the marked items"
      >
        Change status…
      </button>
      <button
        type="button"
        data-testid="selection-action-bar-priority"
        onClick={onChangePriority}
        style={miniButtonStyle}
        title="Change priority for the marked items"
      >
        Change priority…
      </button>
      <button
        type="button"
        data-testid="selection-action-bar-add-to-agent"
        onClick={onAddAllToAgent}
        style={miniButtonStyle}
        title="Add all marked items to the agent's context"
      >
        Add to agent context
      </button>
      <button
        type="button"
        data-testid="selection-action-bar-delete"
        onClick={onDelete}
        disabled={allLocked}
        style={{ ...miniButtonStyle, opacity: allLocked ? 0.4 : 1 }}
        title={allLocked ? "Selected items are locked (in progress)" : "Delete the marked items"}
      >
        Delete
      </button>
    </div>
  );
}

const containerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "5px 10px",
  background: "var(--bg-2)",
  borderBottom: "1px solid var(--border)",
  fontSize: 12,
  color: "var(--fg)",
  position: "sticky",
  top: 0,
  zIndex: 5,
};

const summaryStyle: CSSProperties = {
  fontWeight: 600,
  color: "var(--fg)",
  marginRight: 4,
};

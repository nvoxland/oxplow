import type { CSSProperties } from "react";
import type { FieldDecl, WorkItem } from "../../workItems.js";
import { miniButtonStyle } from "./plan-utils.js";

/**
 * Selection-aware action bar that hovers above the tasks list whenever
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
  /** Marked items (the ones the bar's actions apply to). */
  items: WorkItem[];
  /** The list's own fields: a "Change …" for each editable enum. */
  fields: FieldDecl[];
  onClear(): void;
  onChangeStatus(): void;
  onChangeField(field: FieldDecl): void;
  onAddAllToAgent(): void;
  /** Absent when the list can't delete. */
  onDelete?(): void;
}

export function SelectionActionBar({
  items,
  onClear,
  fields,
  onChangeStatus,
  onChangeField,
  onAddAllToAgent,
  onDelete,
}: SelectionActionBarProps) {
  if (!shouldShowSelectionActionBar(items.length)) return null;
  const lockedCount = items.filter((item) => item.state === "in_progress").length;
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
        title="Change the state of the marked items"
      >
        Change state…
      </button>
      {fields
        .filter((f) => f.kind === "enum" && !f.read_only)
        .map((field) => (
          <button
            key={field.name}
            type="button"
            data-testid={`selection-action-bar-field-${field.name}`}
            onClick={() => onChangeField(field)}
            style={miniButtonStyle}
            title={`Change ${field.title.toLowerCase()} for the marked items`}
          >
            Change {field.title.toLowerCase()}…
          </button>
        ))}
      <button
        type="button"
        data-testid="selection-action-bar-add-to-agent"
        onClick={onAddAllToAgent}
        style={miniButtonStyle}
        title="Add all marked items to the agent's context"
      >
        Add to agent context
      </button>
      {onDelete ? (
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
      ) : null}
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
  fontSize: "var(--text-xs)",
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

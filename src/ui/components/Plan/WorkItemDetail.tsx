import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import type { WorkItem, WorkItemPriority, WorkItemStatus } from "../../api.js";
import { deleteButtonStyle, inputStyle, miniButtonStyle } from "./plan-utils.js";

export interface WorkItemDetailChanges {
  title?: string;
  description?: string;
  acceptanceCriteria?: string | null;
  parentId?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
}

const STATUS_OPTIONS: WorkItemStatus[] = [
  "waiting", "ready", "in_progress", "human_check", "blocked", "done", "canceled",
];
const PRIORITY_OPTIONS: WorkItemPriority[] = ["low", "medium", "high", "urgent"];

/**
 * Expanded view of a work-item row — inline edit of title / description /
 * acceptance, status + priority pickers, delete button. Each field commits
 * on blur or Enter; Escape reverts.
 */
export function WorkItemDetail({
  item,
  onUpdateWorkItem,
  onRequestDelete,
}: {
  item: WorkItem;
  onUpdateWorkItem: (itemId: string, changes: WorkItemDetailChanges) => Promise<void>;
  onRequestDelete(): void;
}) {
  return (
    <div
      style={{ padding: "6px 10px 10px 10px", background: "var(--bg-detail)", borderBottom: "1px solid var(--border)", display: "flex", flexDirection: "column", gap: 6, fontSize: 12 }}
      onClick={(event) => event.stopPropagation()}
    >
      <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center", fontSize: 11 }}>
        <span style={{ color: "var(--muted)" }}>{item.kind}</span>
        <span style={{ color: "var(--muted)" }}>·</span>
        <InlineSelect
          value={item.status}
          options={STATUS_OPTIONS}
          onChange={(value) => void onUpdateWorkItem(item.id, { status: value as WorkItemStatus })}
        />
        <span style={{ color: "var(--muted)" }}>·</span>
        <InlineSelect
          value={item.priority}
          options={PRIORITY_OPTIONS}
          onChange={(value) => void onUpdateWorkItem(item.id, { priority: value as WorkItemPriority })}
          suffix=" priority"
        />
        <span style={{ color: "var(--muted)" }}>·</span>
        <span style={{ color: "var(--muted)" }}>by {item.created_by}</span>
        <span style={{ flex: 1 }} />
        <button type="button" onClick={onRequestDelete} style={deleteButtonStyle} title="Delete work item">Delete</button>
      </div>
      <EditableField
        key={`title-${item.id}-${item.updated_at}`}
        label="Title"
        value={item.title}
        placeholder="Title"
        multiline={false}
        onCommit={(value) => {
          const trimmed = value.trim();
          if (!trimmed || trimmed === item.title) return;
          void onUpdateWorkItem(item.id, { title: trimmed });
        }}
      />
      <EditableField
        key={`desc-${item.id}-${item.updated_at}`}
        label="Description"
        value={item.description}
        placeholder="Add a description…"
        multiline
        onCommit={(value) => {
          if (value === item.description) return;
          void onUpdateWorkItem(item.id, { description: value });
        }}
      />
      <EditableField
        key={`accept-${item.id}-${item.updated_at}`}
        label="Acceptance"
        value={item.acceptance_criteria ?? ""}
        placeholder="Acceptance criteria, one per line"
        multiline
        onCommit={(value) => {
          const next = value.length === 0 ? null : value;
          if (next === item.acceptance_criteria) return;
          void onUpdateWorkItem(item.id, { acceptanceCriteria: next });
        }}
      />
    </div>
  );
}

function EditableField({
  label,
  value,
  placeholder,
  multiline,
  onCommit,
}: {
  label: string;
  value: string;
  placeholder: string;
  multiline: boolean;
  onCommit(value: string): void;
}) {
  const [draft, setDraft] = useState(value);
  const [editing, setEditing] = useState(false);
  // Latch "the user clicked Cancel" across the mousedown → blur → click chain
  // so the blur handler knows to skip auto-commit and revert instead. Using a
  // ref avoids a state update during the mousedown event.
  const cancelRequested = useRef(false);
  const dirty = draft !== value;

  useEffect(() => {
    if (!editing) setDraft(value);
  }, [value, editing]);

  const commit = () => {
    setEditing(false);
    if (draft === value) return;
    onCommit(draft);
  };

  const revert = () => {
    setDraft(value);
    setEditing(false);
  };

  const inputProps = {
    value: draft,
    placeholder,
    onChange: (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setDraft(event.target.value),
    onFocus: () => setEditing(true),
    onBlur: () => {
      if (cancelRequested.current) {
        cancelRequested.current = false;
        revert();
      } else {
        commit();
      }
    },
    onKeyDown: (event: React.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancelRequested.current = true;
        (event.target as HTMLElement).blur();
      } else if (event.key === "Enter" && !multiline) {
        event.preventDefault();
        (event.target as HTMLElement).blur();
      } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        (event.target as HTMLElement).blur();
      }
    },
    style: {
      ...inputStyle,
      width: "100%",
      minHeight: multiline ? 48 : undefined,
      resize: multiline ? ("vertical" as const) : undefined,
      fontFamily: "inherit",
    },
  };

  // Save/Cancel surface while the user is actively editing a dirty draft.
  // Clicking Save would blur the input anyway (→ commit); the button is
  // mostly a visible "here's how to save" affordance. Cancel has to set the
  // cancelRequested latch from mousedown so the blur that follows reverts
  // instead of committing.
  const actions = editing && dirty ? (
    <div style={actionRowStyle}>
      <button
        type="button"
        onMouseDown={(event) => { event.preventDefault(); cancelRequested.current = true; }}
        onClick={revert}
        style={{ ...miniButtonStyle, padding: "3px 10px" }}
        title="Discard changes to this field (Escape)"
      >Cancel</button>
      <button
        type="button"
        onClick={commit}
        style={{ ...miniButtonStyle, padding: "3px 10px", background: "var(--accent)", color: "#fff", borderColor: "var(--accent)" }}
        title={multiline ? "Save changes (Cmd/Ctrl+Enter)" : "Save changes (Enter)"}
      >Save</button>
    </div>
  ) : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--muted)" }}>{label}</div>
      {multiline ? <textarea {...inputProps} /> : <input {...inputProps} />}
      {actions}
    </div>
  );
}

const actionRowStyle: CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: 6,
  marginTop: 2,
};

function InlineSelect({
  value,
  options,
  onChange,
  suffix,
}: {
  value: string;
  options: readonly string[];
  onChange(value: string): void;
  suffix?: string;
}) {
  return (
    <span style={{ position: "relative", display: "inline-block" }}>
      <span style={{ color: "inherit" }}>{value}{suffix ?? ""}</span>
      <select
        value={value}
        onChange={(event) => onChange(event.target.value)}
        style={{
          position: "absolute",
          inset: 0,
          opacity: 0,
          cursor: "pointer",
          width: "100%",
          height: "100%",
          font: "inherit",
        }}
      >
        {options.map((option) => (
          <option key={option} value={option}>{option}</option>
        ))}
      </select>
    </span>
  );
}

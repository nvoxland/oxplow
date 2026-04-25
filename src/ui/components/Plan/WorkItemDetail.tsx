import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import type { EffortDetail, WorkItem, WorkItemPriority, WorkItemStatus, WorkNote } from "../../api.js";
import { MarkdownView } from "../Notes/MarkdownView.js";
import { deleteButtonStyle, inputStyle, miniButtonStyle } from "./plan-utils.js";

/**
 * One entry in the merged Activity timeline rendered inside the work-item
 * modal. Notes (`add_work_note` / `complete_task` summaries) and efforts
 * (start/close windows + changed-file lists) are interleaved into a single
 * chronological list so reviewers see everything that happened on this
 * item in one place — no separate "Notes" / "Efforts" subsections.
 *
 * The list is newest-first. For closed efforts we sort on `ended_at` (the
 * effort *finishing* is the user-visible event); the active effort sorts
 * on `started_at` and gets `active: true` so the renderer can flag it
 * with a subtle "in progress" badge.
 */
export type ActivityRow =
  | { kind: "note"; id: string; timestamp: string; note: WorkNote }
  | { kind: "effort"; id: string; timestamp: string; active: boolean; detail: EffortDetail };

export function buildActivityTimeline(notes: WorkNote[], efforts: EffortDetail[]): ActivityRow[] {
  const rows: ActivityRow[] = [];
  for (const note of notes) {
    rows.push({ kind: "note", id: note.id, timestamp: note.created_at, note });
  }
  for (const detail of efforts) {
    const active = !detail.effort.ended_at;
    const timestamp = detail.effort.ended_at ?? detail.effort.started_at;
    rows.push({ kind: "effort", id: detail.effort.id, timestamp, active, detail });
  }
  rows.sort((a, b) => (a.timestamp < b.timestamp ? 1 : a.timestamp > b.timestamp ? -1 : 0));
  return rows;
}

export interface WorkItemDetailChanges {
  title?: string;
  description?: string;
  acceptanceCriteria?: string | null;
  parentId?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
}

const STATUS_OPTIONS_BASE: WorkItemStatus[] = [
  "blocked", "ready", "human_check", "done", "archived", "canceled",
];
const PRIORITY_OPTIONS: WorkItemPriority[] = ["low", "medium", "high", "urgent"];

function statusOptionsFor(current: WorkItemStatus): WorkItemStatus[] {
  return current === "in_progress" ? [...STATUS_OPTIONS_BASE, "in_progress"] : STATUS_OPTIONS_BASE;
}

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
          options={statusOptionsFor(item.status)}
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
        renderMarkdown
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
        renderMarkdown
        onCommit={(value) => {
          const next = value.length === 0 ? null : value;
          if (next === item.acceptance_criteria) return;
          void onUpdateWorkItem(item.id, { acceptanceCriteria: next });
        }}
      />
    </div>
  );
}

/**
 * Single chronological list (newest first) mixing work-item notes and
 * efforts inside the work-item modal. Replaces the previous two-section
 * layout (Notes pane + separate Efforts pane with an "active effort"
 * callout box) so the timeline reads top-to-bottom without overlap.
 *
 * Active effort renders inline at the top with a subtle "in progress"
 * badge — no callout box.
 */
export function ActivityTimeline({
  notes,
  efforts,
  formatTimestamp,
  onOpenFile,
  onShowInHistory,
}: {
  notes: WorkNote[];
  efforts: EffortDetail[];
  formatTimestamp(iso: string): string;
  onOpenFile?(path: string): void | Promise<void>;
  onShowInHistory?(snapshotId: string): void;
}) {
  const rows = buildActivityTimeline(notes, efforts);
  if (rows.length === 0) {
    return (
      <div style={{ color: "var(--muted)", fontSize: 12, fontStyle: "italic" }}>
        No activity yet — moving this item to "in progress" starts an effort, and notes will land here.
      </div>
    );
  }
  return (
    <div
      data-testid="work-item-activity"
      style={{ display: "flex", flexDirection: "column", gap: 8, overflowY: "auto", border: "1px solid var(--border)", borderRadius: 6, padding: 8, background: "var(--bg-1)" }}
    >
      {rows.map((row) => row.kind === "note" ? (
        <ActivityNoteRow key={`note-${row.id}`} note={row.note} formatTimestamp={formatTimestamp} />
      ) : (
        <ActivityEffortRow
          key={`effort-${row.id}`}
          detail={row.detail}
          active={row.active}
          formatTimestamp={formatTimestamp}
          onOpenFile={onOpenFile}
          onShowInHistory={onShowInHistory}
        />
      ))}
    </div>
  );
}

function ActivityNoteRow({ note, formatTimestamp }: { note: WorkNote; formatTimestamp(iso: string): string }) {
  return (
    <div style={{ fontSize: 12, borderLeft: "2px solid var(--border)", paddingLeft: 8 }}>
      <div style={{ display: "flex", gap: 6, marginBottom: 2, alignItems: "baseline" }}>
        <span style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--muted)", fontWeight: 600 }}>Note</span>
        <span style={{ fontWeight: 600, color: "var(--accent)" }}>{note.author}</span>
        <span style={{ color: "var(--muted)", fontSize: 11 }}>{formatTimestamp(note.created_at)}</span>
      </div>
      {note.body.length > 0 ? (
        <MarkdownView body={note.body} maxHeight={320} />
      ) : (
        <div style={{ color: "var(--muted)", fontStyle: "italic" }}>(empty)</div>
      )}
    </div>
  );
}

function ActivityEffortRow({
  detail,
  active,
  formatTimestamp,
  onOpenFile,
  onShowInHistory,
}: {
  detail: EffortDetail;
  active: boolean;
  formatTimestamp(iso: string): string;
  onOpenFile?(path: string): void | Promise<void>;
  onShowInHistory?(snapshotId: string): void;
}) {
  const endSnapshotId = detail.effort.end_snapshot_id;
  const counts = detail.counts;
  const totalChanged = counts.created + counts.updated + counts.deleted;
  return (
    <div
      data-testid={active ? "work-item-effort-in-progress" : `work-item-effort-${detail.effort.id}`}
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 4,
        borderLeft: `2px solid ${active ? "var(--accent)" : "var(--border)"}`,
        paddingLeft: 8,
      }}
    >
      <div style={{ fontSize: 11, color: "var(--muted)", display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
        <span style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, fontWeight: 600 }}>Effort</span>
        {active ? (
          <span style={{ color: "var(--accent)", fontWeight: 600, fontSize: 10, textTransform: "uppercase", letterSpacing: 0.4 }}>in progress</span>
        ) : null}
        <span>{formatTimestamp(detail.effort.started_at)}</span>
        {detail.effort.ended_at ? <span>→ {formatTimestamp(detail.effort.ended_at)}</span> : null}
        <span style={{ marginLeft: "auto", display: "flex", gap: 6, alignItems: "baseline" }}>
          {counts.created > 0 ? <span style={{ color: "#86efac" }}>+{counts.created}</span> : null}
          {counts.updated > 0 ? <span style={{ color: "#e5a06a" }}>~{counts.updated}</span> : null}
          {counts.deleted > 0 ? <span style={{ color: "#f87171" }}>−{counts.deleted}</span> : null}
          {!active && totalChanged === 0 ? <span>0 files</span> : null}
        </span>
        {onShowInHistory && !active ? (
          <button
            type="button"
            data-testid={`work-item-show-in-history-${detail.effort.id}`}
            onClick={() => { if (endSnapshotId) onShowInHistory(endSnapshotId); }}
            style={{ ...miniButtonStyle, padding: "1px 6px", fontSize: 10 }}
            disabled={!endSnapshotId}
            title={endSnapshotId ? "Open Local History at this effort's end snapshot" : "Effort is still open — no end snapshot yet"}
          >
            In history
          </button>
        ) : null}
      </div>
      {detail.changed_paths.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {detail.changed_paths.map((path) => (
            <div key={path} style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11 }}>
              {onOpenFile ? (
                <button
                  type="button"
                  onClick={() => void onOpenFile(path)}
                  style={{ background: "transparent", border: "none", padding: 0, color: "var(--accent)", cursor: "pointer", textAlign: "left", font: "inherit", textDecoration: "underline", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1, minWidth: 0 }}
                >
                  {path}
                </button>
              ) : (
                <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{path}</span>
              )}
            </div>
          ))}
        </div>
      ) : null}
      {detail.effort.summary && detail.effort.summary.length > 0 ? (
        <div data-testid={`work-item-effort-summary-${detail.effort.id}`} style={{ fontSize: 12 }}>
          <MarkdownView body={detail.effort.summary} maxHeight={240} />
        </div>
      ) : !active ? (
        <div data-testid={`work-item-effort-summary-${detail.effort.id}`} style={{ fontSize: 11, color: "var(--muted)", fontStyle: "italic" }}>
          No summary recorded for this effort.
        </div>
      ) : null}
    </div>
  );
}

function EditableField({
  label,
  value,
  placeholder,
  multiline,
  renderMarkdown = false,
  onCommit,
}: {
  label: string;
  value: string;
  placeholder: string;
  multiline: boolean;
  /**
   * When true and the field is not being edited and the value is non-empty,
   * render the value as markdown (headings, lists, code, links, emphasis)
   * instead of as a plain textarea. Click the rendered surface to edit.
   * Long content gets a max-height + internal scroll so the modal/row
   * doesn't grow unbounded.
   */
  renderMarkdown?: boolean;
  onCommit(value: string): void;
}) {
  const [draft, setDraft] = useState(value);
  const [editing, setEditing] = useState(false);
  // When rendering markdown for the value, the textarea is hidden until the
  // user clicks the rendered surface. `revealEditor` swaps the markdown view
  // for the textarea (which then autoFocuses → setEditing(true)).
  const [revealEditor, setRevealEditor] = useState(false);
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
    setRevealEditor(false);
    if (draft === value) return;
    onCommit(draft);
  };

  const revert = () => {
    setDraft(value);
    setEditing(false);
    setRevealEditor(false);
  };

  // Show the markdown view when the field has rendered content and the
  // user isn't editing yet. Clicking it reveals the editor.
  const showMarkdown = renderMarkdown && multiline && !editing && !revealEditor && value.length > 0;

  const inputProps = {
    value: draft,
    placeholder,
    autoFocus: revealEditor,
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
      {showMarkdown ? (
        <div
          role="button"
          tabIndex={0}
          onClick={() => setRevealEditor(true)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              setRevealEditor(true);
            }
          }}
          title="Click to edit"
          style={markdownSurfaceStyle}
        >
          <MarkdownView body={value} maxHeight={320} />
        </div>
      ) : multiline ? (
        <textarea {...inputProps} />
      ) : (
        <input {...inputProps} />
      )}
      {actions}
    </div>
  );
}

const markdownSurfaceStyle: CSSProperties = {
  border: "1px solid transparent",
  borderRadius: 4,
  padding: "4px 6px",
  cursor: "text",
  background: "transparent",
  fontSize: 12,
  lineHeight: 1.45,
};

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

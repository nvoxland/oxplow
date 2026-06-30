import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { Pencil } from "lucide-react";
import type { EffortDetail, Task, TaskPriority, TaskStatus } from "../../api.js";
import { MarkdownView } from "../Wiki/MarkdownView.js";
import { RichTextField } from "../RichText/RichTextField.js";
import type { RichTextCommentConfig } from "../RichText/RichTextField.js";
import { inputStyle, miniButtonStyle } from "./plan-utils.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef } from "../../tabs/pageRefs.js";
import { ChangeAnalysisFileTree } from "../ChangeAnalysis/FileTreeView.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { DISK, snapshotVersion } from "../../file-version.js";
import { EffortTokenUsageBlock } from "../EffortTokenUsage.js";
import { resolveEffortEndpoints } from "../../diffViewModel.js";
import { diffEndpoints } from "../../api.js";
import type { BranchChangeEntry } from "../../api.js";

/**
 * One entry in the tasks Activity timeline. Each effort
 * (in_progress → done/blocked/canceled cycle) carries a free-form
 * `summary` field that the runtime fills in via `complete_task`, so
 * efforts double as the "what happened on this item" log. Per-item
 * notes were retired — they recorded the same thing.
 */
export type ActivityRow =
  { kind: "effort"; id: string; timestamp: string; active: boolean; detail: EffortDetail };

/**
 * Render an effort's end timestamp. When start and end fall on the
 * same calendar day, the date is implied by the start half — drop it
 * and show only the time, so the user reads `5/15, 5:12 PM → 5:13 PM`
 * instead of repeating the date.
 */
export function formatEffortEnd(
  startedIso: string,
  endedIso: string,
  formatTimestamp: (iso: string) => string,
): string {
  const start = new Date(startedIso);
  const end = new Date(endedIso);
  if (start.toDateString() === end.toDateString()) {
    return end.toLocaleTimeString();
  }
  return formatTimestamp(endedIso);
}

export function buildActivityTimeline(efforts: EffortDetail[]): ActivityRow[] {
  const rows: ActivityRow[] = [];
  for (const detail of efforts) {
    const active = !detail.effort.ended_at;
    const timestamp = detail.effort.ended_at ?? detail.effort.started_at;
    rows.push({ kind: "effort", id: detail.effort.id, timestamp, active, detail });
  }
  rows.sort((a, b) => (a.timestamp < b.timestamp ? 1 : a.timestamp > b.timestamp ? -1 : 0));
  return rows;
}

export interface TaskDetailChanges {
  title?: string;
  description?: string;
  parentId?: string | null;
  status?: TaskStatus;
  priority?: TaskPriority;
}

const STATUS_OPTIONS_BASE: TaskStatus[] = [
  "blocked", "ready", "done", "archived", "canceled",
];
const PRIORITY_OPTIONS: TaskPriority[] = ["low", "medium", "high", "urgent"];

function statusOptionsFor(current: TaskStatus): TaskStatus[] {
  return current === "in_progress" ? [...STATUS_OPTIONS_BASE, "in_progress"] : STATUS_OPTIONS_BASE;
}

/**
 * Body half of the tasks detail view — title + description. Status /
 * priority / category / tags / timestamps / destructive actions live
 * in `TaskDetailRail`. Acceptance criteria are no longer a separate
 * field; agents and users are nudged to include a "## Acceptance
 * criteria" subsection inside the description when it would be
 * helpful (the description is rendered as markdown anyway).
 */
export function TaskDetail({
  item,
  onUpdateTask,
  comments,
}: {
  item: Task;
  onUpdateTask: (itemId: string, changes: TaskDetailChanges) => Promise<void>;
  comments?: RichTextCommentConfig;
}) {
  return (
    <div
      className="task-detail-body"
      style={{ display: "flex", flexDirection: "column", gap: 18 }}
      onClick={(event) => event.stopPropagation()}
    >
      <TitleField
        key={`title-${item.id}`}
        value={item.title}
        onCommit={(value) => {
          const trimmed = value.trim();
          if (!trimmed || trimmed === item.title) return;
          void onUpdateTask(item.id, { title: trimmed });
        }}
      />
      <RichTextField
        key={`desc-${item.id}`}
        value={item.description}
        placeholder="Add a description…"
        style={{ paddingLeft: 0, paddingRight: 22 }}
        comments={comments}
        onCommit={(value) => {
          if (value === item.description) return;
          void onUpdateTask(item.id, { description: value });
        }}
      />
    </div>
  );
}

/**
 * Big inline-editable title for the task page header. Plain auto-sizing
 * textarea styled as an H1; Enter blurs/commits, Escape reverts.
 *
 * Tiptap would be overkill for a single-line text field — and rich
 * formatting on a title is a UX anti-pattern (titles in lists need to
 * be plain strings).
 */
function TitleField({
  value,
  onCommit,
}: {
  value: string;
  onCommit(value: string): void;
}) {
  const [draft, setDraft] = useState(value);
  const cancelRequested = useRef(false);
  useEffect(() => { setDraft(value); }, [value]);
  return (
    <div className="oxplow-rt-field" style={{ position: "relative", paddingLeft: 0, paddingRight: 22, maxWidth: "none", margin: 0 }}>
      <textarea
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="Untitled"
        rows={1}
        onBlur={() => {
          if (cancelRequested.current) {
            cancelRequested.current = false;
            setDraft(value);
            return;
          }
          if (draft.trim() && draft !== value) onCommit(draft);
          else setDraft(value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            cancelRequested.current = true;
            (e.target as HTMLTextAreaElement).blur();
          } else if (e.key === "Enter") {
            e.preventDefault();
            (e.target as HTMLTextAreaElement).blur();
          }
        }}
        className="task-title-field"
        style={{
          width: "100%",
          background: "transparent",
          border: "none",
          outline: "none",
          resize: "none",
          color: "var(--text-primary)",
          fontFamily: "var(--font-ui)",
          fontSize: "var(--text-2xl)",
          fontWeight: "var(--weight-bold)",
          lineHeight: "var(--leading-tight)",
          padding: "4px 0",
          overflow: "hidden",
        }}
        ref={(el) => {
          if (!el) return;
          el.style.height = "auto";
          el.style.height = `${el.scrollHeight}px`;
        }}
      />
      <Pencil
        size={14}
        aria-hidden
        className="oxplow-rt-pencil"
        style={{
          position: "absolute",
          top: 10,
          right: 4,
          color: "var(--text-secondary)",
          opacity: 0.35,
          pointerEvents: "none",
          transition: "opacity 120ms ease",
        }}
      />
    </div>
  );
}

/**
 * Rail half of the tasks detail view — status + priority pickers,
 * category, tags, timestamps, created-by, and bottom action buttons
 * (scope move + delete). Mirror image of `TaskDetail` body fields.
 */
export function TaskDetailRail({
  item,
  onUpdateTask,
  onDelete,
  scopeAction,
  formatTimestamp = (iso) => new Date(iso).toLocaleString(),
}: {
  item: Task;
  onUpdateTask: (itemId: string, changes: TaskDetailChanges) => Promise<void>;
  /** When provided, renders a danger "Delete" button at the bottom of
   *  the rail (mirrors the wiki page's rail Delete). */
  onDelete?: () => void;
  /** Optional scope move (e.g. "Send to backlog" / "Bring to this
   *  thread"); rendered as a neutral button above Delete. */
  scopeAction?: { label: string; run: () => void };
  formatTimestamp?(iso: string): string;
}) {
  return (
    <div
      style={{ display: "flex", flexDirection: "column", gap: 14, fontSize: "var(--text-xs)" }}
      onClick={(event) => event.stopPropagation()}
    >
      <RailPillRow label="Status">
        <PillSelect
          value={item.status}
          options={statusOptionsFor(item.status)}
          color={statusColor(item.status)}
          onChange={(value) => void onUpdateTask(item.id, { status: value as TaskStatus })}
        />
      </RailPillRow>
      <RailPillRow label="Priority">
        <PillSelect
          value={item.priority}
          options={PRIORITY_OPTIONS}
          color={priorityColor(item.priority)}
          onChange={(value) => void onUpdateTask(item.id, { priority: value as TaskPriority })}
        />
      </RailPillRow>

      <div style={{
        display: "flex",
        flexDirection: "column",
        gap: 3,
        borderTop: "1px solid var(--border-subtle)",
        paddingTop: 12,
        marginTop: 4,
        color: "var(--text-muted)",
        fontSize: "var(--text-xs)",
      }}>
        <RailMetaRow label="ID">#{item.id}</RailMetaRow>
        <RailMetaRow label="Created">{formatTimestamp(item.created_at)}</RailMetaRow>
        <RailMetaRow label="Updated">{formatTimestamp(item.updated_at)}</RailMetaRow>
        <RailMetaRow label="By">{item.created_by}</RailMetaRow>
      </div>

      {scopeAction || onDelete ? (
        <div style={{
          display: "flex",
          flexDirection: "column",
          gap: 8,
          borderTop: "1px solid var(--border-subtle)",
          paddingTop: 12,
          marginTop: 4,
        }}>
          {scopeAction ? (
            <button
              type="button"
              data-testid="task-rail-scope-action"
              onClick={scopeAction.run}
              style={{
                textAlign: "left",
                padding: "6px 10px",
                borderRadius: 4,
                border: "1px solid var(--border-subtle)",
                background: "var(--surface-card)",
                color: "var(--text-primary)",
                cursor: "pointer",
                fontSize: "var(--text-xs)",
              }}
            >
              {scopeAction.label}
            </button>
          ) : null}
          {onDelete ? (
            <button
              type="button"
              onClick={onDelete}
              style={{
                textAlign: "left",
                padding: "6px 10px",
                borderRadius: 4,
                border: "1px solid var(--border-subtle)",
                background: "var(--surface-card)",
                color: "var(--severity-critical)",
                cursor: "pointer",
                fontSize: "var(--text-xs)",
              }}
            >
              Delete
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function statusColor(status: TaskStatus): string {
  switch (status) {
    case "in_progress": return "var(--status-running)";
    case "ready": return "var(--status-ready)";
    case "done": return "var(--status-done)";
    case "blocked": return "var(--status-waiting)";
    case "canceled":
    case "archived":
      return "var(--status-canceled)";
    default:
      return "var(--status-ready)";
  }
}

function priorityColor(priority: TaskPriority): string {
  switch (priority) {
    case "urgent": return "var(--priority-urgent)";
    case "high":   return "var(--priority-high)";
    case "medium": return "var(--priority-medium)";
    case "low":    return "var(--priority-low)";
    default:       return "var(--priority-medium)";
  }
}

function RailPillRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{
        textTransform: "uppercase",
        letterSpacing: 0.5,
        fontSize: 10,
        color: "var(--text-secondary)",
        fontWeight: 500,
      }}>{label}</div>
      <div>{children}</div>
    </div>
  );
}

/**
 * Colored pill that opens a native `<select>` on click. The native
 * select stays transparent over the pill so keyboard navigation +
 * accessibility come for free; the pill chrome is purely visual.
 */
function PillSelect({
  value,
  options,
  color,
  onChange,
}: {
  value: string;
  options: readonly string[];
  color: string;
  onChange(value: string): void;
}) {
  return (
    <span
      style={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        padding: "3px 10px",
        borderRadius: 999,
        background: "var(--surface-card)",
        border: `1px solid ${color}`,
        color: "var(--text-primary)",
        fontSize: "var(--text-xs)",
        cursor: "pointer",
        minWidth: 0,
      }}
    >
      <span
        style={{
          width: 8,
          height: 8,
          borderRadius: "50%",
          background: color,
          flexShrink: 0,
        }}
        aria-hidden
      />
      <span>{value.replace(/_/g, " ")}</span>
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
          <option key={option} value={option}>{option.replace(/_/g, " ")}</option>
        ))}
      </select>
    </span>
  );
}

function RailMetaRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", gap: 8 }}>
      <span style={{ width: 56, color: "var(--text-muted)" }}>{label}</span>
      <span style={{ color: "var(--text-secondary)" }}>{children}</span>
    </div>
  );
}


/**
 * Single chronological list (newest first) mixing tasks notes and
 * efforts inside the tasks modal. Replaces the previous two-section
 * layout (Notes pane + separate Efforts pane with an "active effort"
 * callout box) so the timeline reads top-to-bottom without overlap.
 *
 * Active effort renders inline at the top with a subtle "in progress"
 * badge — no callout box.
 */
export function ActivityTimeline({
  efforts,
  formatTimestamp,
  onOpenFile,
  onShowEffortDiff,
  onOpenDiff,
}: {
  efforts: EffortDetail[];
  formatTimestamp(iso: string): string;
  onOpenFile?(path: string): void | Promise<void>;
  /** Open the effort's diff view (start→end snapshot bracket). */
  onShowEffortDiff?(effortId: string): void;
  /** Open a diff tab. Modified-file rows use this to show the file's
   *  diff across the effort's snapshot bracket. */
  onOpenDiff?(spec: DiffSpec): void;
}) {
  const rows = buildActivityTimeline(efforts);
  if (rows.length === 0) {
    return (
      <div
        data-testid="tasks-activity"
        style={{
          color: "var(--text-muted)",
          fontSize: "var(--text-sm)",
          fontStyle: "italic",
        }}
      >
        No recorded effort
      </div>
    );
  }
  return (
    <div
      data-testid="tasks-activity"
      style={{ display: "flex", flexDirection: "column", gap: 28 }}
    >
      {rows.map((row) =>
        // An in-progress effort shows ONLY "In progress" — no changed
        // files, tests, coverage, or summary until it closes. The full
        // breakdown (and the underlying snapshot/observation fetches) is
        // reserved for completed efforts.
        row.active ? (
          <ActiveEffortSection key={`effort-${row.id}`} />
        ) : (
          <ActivityEffortSection
            key={`effort-${row.id}`}
            detail={row.detail}
            formatTimestamp={formatTimestamp}
            onOpenFile={onOpenFile}
            onShowEffortDiff={onShowEffortDiff}
            onOpenDiff={onOpenDiff}
          />
        ),
      )}
    </div>
  );
}

/**
 * A still-open effort. We deliberately show ONLY an "In progress" marker
 * — no changed files, tests, coverage, or summary — until the effort
 * closes. Mid-effort those numbers are noisy and incomplete (the snapshot
 * bracket has no end yet, runs are still landing), so they're reserved for
 * the completed view. No data fetching here on purpose.
 */
function ActiveEffortSection() {
  return (
    <section
      data-testid="tasks-effort-in-progress"
      style={{
        border: "1px solid var(--border-subtle)",
        borderRadius: 8,
        overflow: "hidden",
        background: "var(--surface-card)",
      }}
    >
      <header
        style={{
          padding: "8px 14px",
          background: "var(--panel-header-bg)",
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.4,
            color: "var(--accent)",
          }}
        >
          In progress
        </h3>
      </header>
    </section>
  );
}

/**
 * One completed effort rendered as a page section: time-range subheader,
 * summary note (markdown), changed-files list, observations, and a
 * "Details" link to the effort diff. No bordered card — these read as part
 * of the page. (In-progress efforts use {@link ActiveEffortSection}.)
 */
function ActivityEffortSection({
  detail,
  formatTimestamp,
  onOpenFile,
  onShowEffortDiff,
  onOpenDiff,
}: {
  detail: EffortDetail;
  formatTimestamp(iso: string): string;
  onOpenFile?(path: string): void | Promise<void>;
  onShowEffortDiff?(effortId: string): void;
  onOpenDiff?(spec: DiffSpec): void;
}) {
  const ctxNav = useOptionalPageNavigation();
  const openFile = (path: string) => {
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else void onOpenFile?.(path);
  };
  // Open the file's diff across this effort's snapshot bracket
  // (start → end; falls back to the working tree when a pin is missing
  // — e.g. an active effort with no end snapshot yet).
  const openDiff = (path: string) => {
    if (!onOpenDiff) return;
    const left = detail.effort.start_snapshot_id;
    const right = detail.effort.end_snapshot_id;
    onOpenDiff({
      path,
      leftVersion: left ? snapshotVersion(left) : DISK,
      rightVersion: right ? snapshotVersion(right) : DISK,
      baseLabel: "effort changes",
    });
  };
  // Per-file status + churn for the effort's snapshot bracket, so the
  // Modified Files list can use the same rich tree as the diff view (A/M/D
  // + churn) instead of bare filenames. Mapped onto `changed_paths` (the
  // effort's claimed files); falls back to status "modified"/0 churn until
  // the diff resolves.
  const [diffByPath, setDiffByPath] = useState<
    Map<string, { status: string; additions: number; deletions: number }>
  >(new Map());
  useEffect(() => {
    let cancelled = false;
    const startId = detail.effort.start_snapshot_id;
    const endId = detail.effort.end_snapshot_id;
    const { start, end } = resolveEffortEndpoints({
      startSnapshotId: startId != null ? Number(startId) : null,
      endSnapshotId: endId != null ? Number(endId) : null,
    });
    void diffEndpoints(start, end)
      .then((entries) => {
        if (cancelled) return;
        setDiffByPath(
          new Map(
            entries.map((e) => [
              e.path,
              { status: e.status, additions: e.additions, deletions: e.deletions },
            ]),
          ),
        );
      })
      .catch(() => {
        if (!cancelled) setDiffByPath(new Map());
      });
    return () => {
      cancelled = true;
    };
  }, [detail.effort.start_snapshot_id, detail.effort.end_snapshot_id]);
  const effortFiles = useMemo<BranchChangeEntry[]>(
    () =>
      detail.changed_paths.map((path) => {
        const e = diffByPath.get(path);
        return {
          path,
          status: (e?.status ?? "modified") as BranchChangeEntry["status"],
          additions: e?.additions ?? 0,
          deletions: e?.deletions ?? 0,
        };
      }),
    [detail.changed_paths, diffByPath],
  );

  const subheader = `${formatTimestamp(detail.effort.started_at)} → ${formatEffortEnd(detail.effort.started_at, detail.effort.ended_at!, formatTimestamp)}`;
  return (
    <section
      data-testid={`tasks-effort-${detail.effort.id}`}
      style={{
        display: "flex",
        flexDirection: "column",
        border: "1px solid var(--border-subtle)",
        borderRadius: 8,
        overflow: "hidden",
        background: "var(--surface-card)",
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "baseline",
          justifyContent: "space-between",
          gap: 12,
          padding: "8px 14px",
          // Standard panel-header chrome: muted-accent tint (not grey) so
          // the band reads as an intentional header, matching RailSection /
          // Page DetailsBody.
          background: "var(--panel-header-bg)",
          borderBottom: "1px solid var(--border-subtle)",
        }}
      >
        <h3 style={{
          margin: 0,
          fontSize: 11,
          fontWeight: 600,
          textTransform: "uppercase",
          letterSpacing: 0.4,
          color: "var(--text-secondary)",
        }}>
          {subheader}
        </h3>
        {onShowEffortDiff ? (
          <button
            type="button"
            data-testid={`tasks-show-in-history-${detail.effort.id}`}
            onClick={() => onShowEffortDiff(detail.effort.id)}
            style={{
              flexShrink: 0,
              background: "transparent",
              border: "none",
              padding: 0,
              color: "var(--accent)",
              cursor: "pointer",
              font: "inherit",
              fontSize: "var(--text-xs)",
              textDecoration: "underline",
            }}
            title="Open the diff view for this effort (start → end)"
          >
            Details
          </button>
        ) : null}
      </header>
      <div style={{ display: "flex", flexDirection: "column", gap: 10, padding: "12px 14px" }}>
      {detail.effort.summary && detail.effort.summary.length > 0 ? (
        <div data-testid={`tasks-effort-summary-${detail.effort.id}`}>
          <MarkdownView body={detail.effort.summary} />
        </div>
      ) : (
        <div
          data-testid={`tasks-effort-summary-${detail.effort.id}`}
          style={{ fontSize: 12, color: "var(--text-muted)", fontStyle: "italic" }}
        >
          No summary recorded.
        </div>
      )}
      {detail.changed_paths.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <h2 className="task-activity-heading">Modified Files</h2>
          <ChangeAnalysisFileTree
            files={effortFiles}
            target={`effort:${detail.effort.id}`}
            onOpenFile={(path) => openFile(path)}
            onOpenFileDiff={onOpenDiff ? (path) => openDiff(path) : undefined}
            showFileCount={false}
          />
        </div>
      ) : null}
        {/* Coverage / tests / static-analysis are intentionally NOT shown
            here — the task-page effort section stays focused on what changed.
            That breakdown lives on the effort diff view (DiffViewPage). Token
            usage stays: it's effort metadata, not test information. It always
            reports its status here (showWhenEmpty) — "No token data collected"
            rather than vanishing. */}
        <EffortTokenUsageBlock effortId={detail.effort.id} showWhenEmpty />
      </div>
    </section>
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
  fontSize: "var(--text-xs)",
  lineHeight: 1.45,
};

const actionRowStyle: CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: 6,
  marginTop: 2,
};


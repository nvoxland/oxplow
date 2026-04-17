import type { CSSProperties, ReactNode } from "react";
import { useMemo, useState } from "react";
import type {
  Stream,
  Batch,
  BatchWorkState,
  WorkItem,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
} from "../api.js";
import type { OpenFileState } from "../../session/file-session.js";
import type { EditorNavigationTarget } from "../lsp.js";
import { TerminalPane } from "./TerminalPane.js";
import { EditorPane } from "./EditorPane.js";

export type TabId = "agent" | "plan" | "editor";

interface Props {
  stream: Stream;
  batch: Batch | null;
  activeBatchId: string | null;
  batchWork: BatchWorkState | null;
  active: TabId;
  onActiveChange(tab: TabId): void;
  onCreateWorkItem(input: {
    kind: WorkItemKind;
    title: string;
    description?: string;
    parentId?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
  }): Promise<void>;
  onUpdateWorkItem(
    itemId: string,
    changes: {
      title?: string;
      description?: string;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): Promise<void>;
  onDeleteWorkItem(itemId: string): Promise<void>;
  onReorderWorkItems(orderedItemIds: string[]): Promise<void>;
  openFileOrder: string[];
  openFiles: Record<string, OpenFileState>;
  currentFilePath: string | null;
  currentFileContent: string;
  currentFileDirty: boolean;
  onEditorChange(value: string): void;
  onEditorSave(): void;
  editorFindRequest: number;
  editorNavigationTarget: EditorNavigationTarget | null;
  onNavigateToLocation(target: EditorNavigationTarget): Promise<void>;
  onSelectOpenFile(path: string): void;
  onCloseOpenFile(path: string): void;
}

export function MainTabs({
  stream,
  batch,
  activeBatchId,
  batchWork,
  active,
  onActiveChange,
  onCreateWorkItem,
  onUpdateWorkItem,
  onDeleteWorkItem,
  onReorderWorkItems,
  openFileOrder,
  openFiles,
  currentFilePath,
  currentFileContent,
  currentFileDirty,
  onEditorChange,
  onEditorSave,
  editorFindRequest,
  editorNavigationTarget,
  onNavigateToLocation,
  onSelectOpenFile,
  onCloseOpenFile,
}: Props) {
  const tabs: { id: TabId; label: string }[] = [
    { id: "agent", label: batch ? batch.title : "Agent" },
    { id: "plan", label: "Plan" },
    { id: "editor", label: "Editor" },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", borderBottom: "1px solid var(--border)", background: "var(--bg-2)" }}>
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => onActiveChange(t.id)}
            style={{
              padding: "8px 16px",
              background: active === t.id ? "var(--bg)" : "transparent",
              color: active === t.id ? "var(--fg)" : "var(--muted)",
              border: "none",
              borderRight: "1px solid var(--border)",
              borderBottom: active === t.id ? "2px solid var(--accent)" : "2px solid transparent",
              cursor: "pointer",
              fontFamily: "inherit",
              fontSize: 12,
            }}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0, position: "relative" }}>
        <PaneHost visible={active === "agent"}>
          {batch ? (
            <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
              <div style={{ padding: "6px 10px", borderBottom: "1px solid var(--border)", color: "var(--muted)", fontSize: 11 }}>
                {batch.id === activeBatchId
                  ? "Active batch — edits in this session affect the stream worktree."
                  : "Queued batch — use this session for planning and questions before promotion."}
              </div>
              <div style={{ flex: 1, minHeight: 0 }}>
                <TerminalPane paneTarget={batch.pane_target} visible={active === "agent"} />
              </div>
            </div>
          ) : (
            <div style={{ padding: 12, color: "var(--muted)" }}>No batch selected.</div>
          )}
        </PaneHost>
        <PaneHost visible={active === "plan"}>
          <PlanPane
            batch={batch}
            batchWork={batchWork}
            onCreateWorkItem={onCreateWorkItem}
            onUpdateWorkItem={onUpdateWorkItem}
            onDeleteWorkItem={onDeleteWorkItem}
            onReorderWorkItems={onReorderWorkItems}
          />
        </PaneHost>
        <PaneHost visible={active === "editor"}>
          <EditorPane
            stream={stream}
            filePath={currentFilePath}
            value={currentFileContent}
            isDirty={currentFileDirty}
            onChange={onEditorChange}
            onSave={onEditorSave}
            findRequest={editorFindRequest}
            navigationTarget={editorNavigationTarget}
            onNavigateToLocation={onNavigateToLocation}
            openFileOrder={openFileOrder}
            openFiles={openFiles}
            onSelectOpenFile={onSelectOpenFile}
            onCloseOpenFile={onCloseOpenFile}
          />
        </PaneHost>
      </div>
    </div>
  );
}

function PlanPane({
  batch,
  batchWork,
  onCreateWorkItem,
  onUpdateWorkItem,
  onDeleteWorkItem,
  onReorderWorkItems,
}: {
  batch: Batch | null;
  batchWork: BatchWorkState | null;
  onCreateWorkItem: Props["onCreateWorkItem"];
  onUpdateWorkItem: Props["onUpdateWorkItem"];
  onDeleteWorkItem: Props["onDeleteWorkItem"];
  onReorderWorkItems: Props["onReorderWorkItems"];
}) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [kind, setKind] = useState<WorkItemKind>("task");
  const [priority, setPriority] = useState<WorkItemPriority>("medium");
  const [parentId, setParentId] = useState<string>("");
  const [showCompleted, setShowCompleted] = useState(true);
  const epics = batchWork?.epics ?? [];

  const waitingGroups = useMemo(() => groupByEpic(batchWork?.waiting ?? [], epics), [batchWork?.waiting, epics]);
  const progressGroups = useMemo(() => groupByEpic(batchWork?.inProgress ?? [], epics), [batchWork?.inProgress, epics]);
  const doneGroups = useMemo(() => groupByEpic(batchWork?.done ?? [], epics), [batchWork?.done, epics]);

  const total = batchWork?.items.length ?? 0;
  const completed = batchWork?.done.length ?? 0;
  const percent = total === 0 ? 0 : Math.round((completed / total) * 100);

  if (!batch) {
    return <div style={{ padding: 12, color: "var(--muted)" }}>No batch selected.</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      <div style={{ padding: 12, borderBottom: "1px solid var(--border)", display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <div style={{ fontSize: 14, fontWeight: 600 }}>{batch.title}</div>
          <ProgressBadge completed={completed} total={total} percent={percent} />
        </div>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <div style={{ color: "var(--muted)", fontSize: 12 }}>Track epics and tasks for this batch only.</div>
          <label style={{ display: "inline-flex", alignItems: "center", gap: 6, color: "var(--muted)", fontSize: 12, cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={showCompleted}
              onChange={(event) => setShowCompleted(event.target.checked)}
            />
            Show completed
          </label>
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "1.3fr 1fr 0.9fr 1fr auto", gap: 8 }}>
          <input
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            placeholder="New epic or task title"
            style={inputStyle}
          />
          <select value={kind} onChange={(event) => setKind(event.target.value as WorkItemKind)} style={inputStyle}>
            <option value="epic">Epic</option>
            <option value="task">Task</option>
            <option value="subtask">Subtask</option>
            <option value="bug">Bug</option>
            <option value="note">Note</option>
          </select>
          <select value={priority} onChange={(event) => setPriority(event.target.value as WorkItemPriority)} style={inputStyle}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
            <option value="urgent">Urgent</option>
          </select>
          <select value={parentId} onChange={(event) => setParentId(event.target.value)} style={inputStyle}>
            <option value="">No parent epic</option>
            {epics.map((epic) => (
              <option key={epic.id} value={epic.id}>{epic.title}</option>
            ))}
          </select>
          <button
            style={buttonStyle}
            onClick={() => {
              const nextTitle = title.trim();
              if (!nextTitle) return;
              void onCreateWorkItem({
                kind,
                title: nextTitle,
                description,
                parentId: parentId || undefined,
                priority,
                status: kind === "epic" ? "in_progress" : "waiting",
              }).then(() => {
                setTitle("");
                setDescription("");
                setParentId("");
                setKind("task");
                setPriority("medium");
              });
            }}
          >
            Add
          </button>
        </div>
        <textarea
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          placeholder="Optional description"
          style={{ ...inputStyle, minHeight: 56, resize: "vertical" }}
        />
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 12 }}>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: showCompleted ? "repeat(3, minmax(0, 1fr))" : "repeat(2, minmax(0, 1fr))",
            gap: 12,
          }}
        >
          <WorkColumn
            title="Waiting"
            groups={waitingGroups}
            onUpdateWorkItem={onUpdateWorkItem}
            onDeleteWorkItem={onDeleteWorkItem}
            onReorderWorkItems={onReorderWorkItems}
          />
          <WorkColumn
            title="In Progress"
            groups={progressGroups}
            onUpdateWorkItem={onUpdateWorkItem}
            onDeleteWorkItem={onDeleteWorkItem}
            onReorderWorkItems={onReorderWorkItems}
          />
          {showCompleted ? (
            <WorkColumn
              title="Done"
              groups={doneGroups}
              onUpdateWorkItem={onUpdateWorkItem}
              onDeleteWorkItem={onDeleteWorkItem}
              onReorderWorkItems={onReorderWorkItems}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}

function ProgressBadge({ completed, total, percent }: { completed: number; total: number; percent: number }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10, minWidth: 220 }}>
      <div
        style={{
          flex: 1,
          height: 6,
          borderRadius: 999,
          background: "var(--bg-2)",
          border: "1px solid var(--border)",
          overflow: "hidden",
          minWidth: 120,
        }}
      >
        <div
          style={{
            width: `${percent}%`,
            height: "100%",
            background: "var(--accent)",
            transition: "width 120ms ease-out",
          }}
        />
      </div>
      <div style={{ color: "var(--muted)", fontSize: 12, whiteSpace: "nowrap" }}>
        {percent}% · {completed}/{total}
      </div>
    </div>
  );
}

function WorkColumn({
  title,
  groups,
  onUpdateWorkItem,
  onDeleteWorkItem,
  onReorderWorkItems,
}: {
  title: string;
  groups: Array<{ epic: WorkItem | null; items: WorkItem[] }>;
  onUpdateWorkItem: Props["onUpdateWorkItem"];
  onDeleteWorkItem: Props["onDeleteWorkItem"];
  onReorderWorkItems: Props["onReorderWorkItems"];
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10, minWidth: 0 }}>
      <div style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6 }}>{title}</div>
      {groups.length === 0 ? <div style={{ color: "var(--muted)", fontSize: 12 }}>No items.</div> : null}
      {groups.map((group, index) => (
        <WorkGroup
          key={group.epic?.id ?? `root-${index}`}
          group={group}
          onUpdateWorkItem={onUpdateWorkItem}
          onDeleteWorkItem={onDeleteWorkItem}
          onReorderWorkItems={onReorderWorkItems}
        />
      ))}
    </div>
  );
}

function WorkGroup({
  group,
  onUpdateWorkItem,
  onDeleteWorkItem,
  onReorderWorkItems,
}: {
  group: { epic: WorkItem | null; items: WorkItem[] };
  onUpdateWorkItem: Props["onUpdateWorkItem"];
  onDeleteWorkItem: Props["onDeleteWorkItem"];
  onReorderWorkItems: Props["onReorderWorkItems"];
}) {
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);

  const handleDrop = (targetId: string) => {
    if (!draggingId || draggingId === targetId) {
      setDraggingId(null);
      setOverId(null);
      return;
    }
    // Reorder the FULL sibling group on disk — the backend assigns sort_index
    // to every sibling, so we need to send the complete ordered id list for
    // the parent (which is group.epic?.id ?? null). The column-filtered
    // `group.items` may be a subset (only the in-progress items, say), so we
    // cannot build the ordering from it alone. Instead, we drag within the
    // visible subset and the backend re-anchors siblings that share the same
    // parent against that subset's new positions.
    const ids = group.items.map((item) => item.id);
    const from = ids.indexOf(draggingId);
    const to = ids.indexOf(targetId);
    if (from < 0 || to < 0) {
      setDraggingId(null);
      setOverId(null);
      return;
    }
    const next = ids.slice();
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved!);
    setDraggingId(null);
    setOverId(null);
    void onReorderWorkItems(next);
  };

  return (
    <div style={{ border: "1px solid var(--border)", borderRadius: 8, background: "var(--bg-2)", padding: 10 }}>
      {group.epic ? (
        <div style={{ marginBottom: 8, paddingBottom: 8, borderBottom: "1px solid var(--border)" }}>
          <div style={{ fontWeight: 600 }}>{group.epic.title}</div>
          <div style={{ color: "var(--muted)", fontSize: 11 }}>
            {group.epic.kind} · {group.epic.priority} · {group.epic.status}
          </div>
        </div>
      ) : null}
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {group.items.map((item) => (
          <div
            key={item.id}
            draggable
            onDragStart={(event) => {
              setDraggingId(item.id);
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", item.id);
            }}
            onDragEnd={() => {
              setDraggingId(null);
              setOverId(null);
            }}
            onDragOver={(event) => {
              if (!draggingId || draggingId === item.id) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              if (overId !== item.id) setOverId(item.id);
            }}
            onDragLeave={() => {
              if (overId === item.id) setOverId(null);
            }}
            onDrop={(event) => {
              event.preventDefault();
              handleDrop(item.id);
            }}
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 6,
              padding: 6,
              borderRadius: 6,
              border: overId === item.id && draggingId !== item.id
                ? "1px dashed var(--accent)"
                : "1px solid transparent",
              background: draggingId === item.id ? "rgba(255,255,255,0.04)" : "transparent",
              cursor: "grab",
            }}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: 8 }}>
              <div style={{ fontWeight: 500, wordBreak: "break-word" }}>{item.title}</div>
              <div style={{ display: "flex", alignItems: "center", gap: 6, flexShrink: 0 }}>
                <span style={badgeStyle}>{item.priority}</span>
                <button
                  onClick={() => {
                    if (!window.confirm(`Delete "${item.title}"?`)) return;
                    void onDeleteWorkItem(item.id);
                  }}
                  title="Delete work item"
                  aria-label="Delete work item"
                  style={trashButtonStyle}
                >
                  🗑
                </button>
              </div>
            </div>
            <div style={{ color: "var(--muted)", fontSize: 11 }}>
              {item.kind} · {item.status}
            </div>
            {item.description ? <div style={{ fontSize: 12, color: "var(--muted)" }}>{item.description}</div> : null}
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
              {item.status !== "waiting" ? (
                <button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "waiting" })}>Waiting</button>
              ) : null}
              {item.status !== "in_progress" ? (
                <button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "in_progress" })}>Start</button>
              ) : null}
              {item.status !== "done" ? (
                <button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "done" })}>Done</button>
              ) : null}
              {item.status !== "blocked" ? (
                <button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "blocked" })}>Block</button>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function groupByEpic(items: WorkItem[], epics: WorkItem[]) {
  const epicMap = new Map(epics.map((epic) => [epic.id, epic]));
  const grouped = new Map<string, { epic: WorkItem | null; items: WorkItem[] }>();
  for (const item of items) {
    if (item.kind === "epic") {
      grouped.set(item.id, { epic: item, items: [] });
      continue;
    }
    const epic = item.parent_id ? epicMap.get(item.parent_id) ?? null : null;
    const key = epic?.id ?? "__root__";
    const group = grouped.get(key) ?? { epic, items: [] };
    group.items.push(item);
    grouped.set(key, group);
  }
  const result = [...grouped.values()];
  return result.sort((a, b) => (a.epic?.sort_index ?? -1) - (b.epic?.sort_index ?? -1));
}

function PaneHost({ visible, children }: { visible: boolean; children: ReactNode }) {
  return (
    <div
      style={{
        position: "absolute",
        inset: 0,
        display: visible ? "block" : "none",
      }}
    >
      {children}
    </div>
  );
}

const inputStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  font: "inherit",
  padding: "6px 8px",
};

const buttonStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--accent)",
  color: "#fff",
  cursor: "pointer",
  font: "inherit",
  padding: "6px 10px",
};

const miniButtonStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  cursor: "pointer",
  font: "inherit",
  padding: "4px 8px",
  fontSize: 11,
};

const trashButtonStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid transparent",
  background: "transparent",
  color: "var(--muted)",
  cursor: "pointer",
  font: "inherit",
  fontSize: 13,
  lineHeight: 1,
  padding: "2px 6px",
};

const badgeStyle: CSSProperties = {
  borderRadius: 999,
  border: "1px solid var(--border)",
  color: "var(--muted)",
  fontSize: 10,
  padding: "2px 6px",
  textTransform: "uppercase",
  letterSpacing: 0.4,
};

import type { CSSProperties } from "react";
import { useMemo, useState } from "react";
import type {
  Batch,
  BatchWorkState,
  WorkItem,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
} from "../../api.js";

interface Props {
  batch: Batch | null;
  batchWork: BatchWorkState | null;
  onCreateWorkItem(input: {
    kind: WorkItemKind;
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    parentId?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
  }): Promise<void>;
  onUpdateWorkItem(
    itemId: string,
    changes: {
      title?: string;
      description?: string;
      acceptanceCriteria?: string | null;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): Promise<void>;
  onDeleteWorkItem(itemId: string): Promise<void>;
  onReorderWorkItems(orderedItemIds: string[]): Promise<void>;
}

export function PlanPane({
  batch,
  batchWork,
  onCreateWorkItem,
  onUpdateWorkItem,
  onDeleteWorkItem,
  onReorderWorkItems,
}: Props) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [acceptance, setAcceptance] = useState("");
  const [kind, setKind] = useState<WorkItemKind>("task");
  const [priority, setPriority] = useState<WorkItemPriority>("medium");
  const [parentId, setParentId] = useState<string>("");
  const [showCompleted, setShowCompleted] = useState(true);
  const [createOpen, setCreateOpen] = useState(false);
  const epics = batchWork?.epics ?? [];

  const waitingGroups = useMemo(() => groupByEpic(batchWork?.waiting ?? [], epics), [batchWork?.waiting, epics]);
  const progressGroups = useMemo(() => groupByEpic(batchWork?.inProgress ?? [], epics), [batchWork?.inProgress, epics]);
  const doneGroups = useMemo(() => groupByEpic(batchWork?.done ?? [], epics), [batchWork?.done, epics]);

  if (!batch) {
    return <div style={{ padding: 12, color: "var(--muted)" }}>No batch selected.</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      <div style={{ padding: 8, borderBottom: "1px solid var(--border)", display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
          <button onClick={() => setCreateOpen((v) => !v)} style={{ ...miniButtonStyle, padding: "4px 10px" }}>
            {createOpen ? "− Hide form" : "+ New work item"}
          </button>
          <label style={{ display: "inline-flex", alignItems: "center", gap: 6, color: "var(--muted)", fontSize: 11, cursor: "pointer" }}>
            <input type="checkbox" checked={showCompleted} onChange={(event) => setShowCompleted(event.target.checked)} />
            Show completed
          </label>
        </div>
        {createOpen ? (
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
            <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" style={inputStyle} />
            <select value={kind} onChange={(e) => setKind(e.target.value as WorkItemKind)} style={inputStyle}>
              <option value="epic">Epic</option>
              <option value="task">Task</option>
              <option value="subtask">Subtask</option>
              <option value="bug">Bug</option>
              <option value="note">Note</option>
            </select>
            <select value={priority} onChange={(e) => setPriority(e.target.value as WorkItemPriority)} style={inputStyle}>
              <option value="low">Low</option>
              <option value="medium">Medium</option>
              <option value="high">High</option>
              <option value="urgent">Urgent</option>
            </select>
            <select value={parentId} onChange={(e) => setParentId(e.target.value)} style={inputStyle}>
              <option value="">No parent epic</option>
              {epics.map((epic) => (<option key={epic.id} value={epic.id}>{epic.title}</option>))}
            </select>
            <textarea value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Description (the approach)" style={{ ...inputStyle, gridColumn: "1 / 3", minHeight: 48, resize: "vertical" }} />
            <textarea value={acceptance} onChange={(e) => setAcceptance(e.target.value)} placeholder="Acceptance criteria, one per line" style={{ ...inputStyle, gridColumn: "1 / 3", minHeight: 48, resize: "vertical" }} />
            <button
              style={{ ...buttonStyle, gridColumn: "1 / 3" }}
              onClick={() => {
                const nextTitle = title.trim();
                if (!nextTitle) return;
                void onCreateWorkItem({
                  kind,
                  title: nextTitle,
                  description,
                  acceptanceCriteria: acceptance || null,
                  parentId: parentId || undefined,
                  priority,
                  status: kind === "epic" ? "in_progress" : "waiting",
                }).then(() => {
                  setTitle(""); setDescription(""); setAcceptance(""); setParentId(""); setKind("task"); setPriority("medium");
                });
              }}
            >Add</button>
          </div>
        ) : null}
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 8 }}>
        <div style={{ display: "grid", gridTemplateColumns: showCompleted ? "repeat(3, minmax(0, 1fr))" : "repeat(2, minmax(0, 1fr))", gap: 8 }}>
          <WorkColumn title="Waiting" groups={waitingGroups} onUpdateWorkItem={onUpdateWorkItem} onDeleteWorkItem={onDeleteWorkItem} onReorderWorkItems={onReorderWorkItems} />
          <WorkColumn title="In Progress" groups={progressGroups} onUpdateWorkItem={onUpdateWorkItem} onDeleteWorkItem={onDeleteWorkItem} onReorderWorkItems={onReorderWorkItems} />
          {showCompleted ? (
            <WorkColumn title="Done" groups={doneGroups} onUpdateWorkItem={onUpdateWorkItem} onDeleteWorkItem={onDeleteWorkItem} onReorderWorkItems={onReorderWorkItems} />
          ) : null}
        </div>
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
        <WorkGroup key={group.epic?.id ?? `root-${index}`} group={group} onUpdateWorkItem={onUpdateWorkItem} onDeleteWorkItem={onDeleteWorkItem} onReorderWorkItems={onReorderWorkItems} />
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
      setDraggingId(null); setOverId(null); return;
    }
    const ids = group.items.map((item) => item.id);
    const from = ids.indexOf(draggingId);
    const to = ids.indexOf(targetId);
    if (from < 0 || to < 0) {
      setDraggingId(null); setOverId(null); return;
    }
    const next = ids.slice();
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved!);
    setDraggingId(null); setOverId(null);
    void onReorderWorkItems(next);
  };

  return (
    <div style={{ border: "1px solid var(--border)", borderRadius: 8, background: "var(--bg-2)", padding: 8 }}>
      {group.epic ? (
        <div style={{ marginBottom: 8, paddingBottom: 8, borderBottom: "1px solid var(--border)" }}>
          <div style={{ fontWeight: 600 }}>{group.epic.title}</div>
          <div style={{ color: "var(--muted)", fontSize: 11 }}>{group.epic.kind} · {group.epic.priority} · {group.epic.status}</div>
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
            onDragEnd={() => { setDraggingId(null); setOverId(null); }}
            onDragOver={(event) => {
              if (!draggingId || draggingId === item.id) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              if (overId !== item.id) setOverId(item.id);
            }}
            onDragLeave={() => { if (overId === item.id) setOverId(null); }}
            onDrop={(event) => { event.preventDefault(); handleDrop(item.id); }}
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 6,
              padding: 6,
              borderRadius: 6,
              border: overId === item.id && draggingId !== item.id ? "1px dashed var(--accent)" : "1px solid transparent",
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
                >🗑</button>
              </div>
            </div>
            <div style={{ color: "var(--muted)", fontSize: 11 }}>{item.kind} · {item.status}</div>
            {item.description ? <div style={{ fontSize: 12, color: "var(--muted)" }}>{item.description}</div> : null}
            {item.acceptance_criteria ? (
              <div style={{ fontSize: 11, color: "var(--muted)", borderLeft: "2px solid var(--border)", paddingLeft: 8 }}>
                <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, marginBottom: 2 }}>Acceptance</div>
                <div style={{ whiteSpace: "pre-wrap" }}>{item.acceptance_criteria}</div>
              </div>
            ) : null}
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
              {item.status !== "waiting" ? (<button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "waiting" })}>Waiting</button>) : null}
              {item.status !== "in_progress" ? (<button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "in_progress" })}>Start</button>) : null}
              {item.status !== "done" ? (<button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "done" })}>Done</button>) : null}
              {item.status !== "blocked" ? (<button style={miniButtonStyle} onClick={() => void onUpdateWorkItem(item.id, { status: "blocked" })}>Block</button>) : null}
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

const inputStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "inherit", font: "inherit", padding: "4px 6px", fontSize: 12,
};

const buttonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--accent)", color: "#fff", cursor: "pointer", font: "inherit", padding: "6px 10px",
};

const miniButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "inherit", cursor: "pointer", font: "inherit", padding: "3px 6px", fontSize: 11,
};

const trashButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid transparent", background: "transparent", color: "var(--muted)", cursor: "pointer", font: "inherit", fontSize: 13, lineHeight: 1, padding: "2px 6px",
};

const badgeStyle: CSSProperties = {
  borderRadius: 999, border: "1px solid var(--border)", color: "var(--muted)", fontSize: 10, padding: "1px 6px", textTransform: "uppercase", letterSpacing: 0.4,
};

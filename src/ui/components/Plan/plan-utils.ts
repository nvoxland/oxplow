import type { CSSProperties } from "react";
import type {
  BacklogState,
  BatchWorkState,
  WorkItem,
  WorkItemPriority,
  WorkItemStatus,
} from "../../api.js";

export interface WorkItemGroup {
  epic: WorkItem | null;
  items: WorkItem[];
}

export function buildBacklogGroups(state: BacklogState | null, showCompleted: boolean): WorkItemGroup[] {
  if (!state) return [];
  const items = showCompleted
    ? [...state.waiting, ...state.inProgress, ...state.done]
    : [...state.waiting, ...state.inProgress];
  if (items.length === 0) return [];
  items.sort((a, b) => a.sort_index - b.sort_index);
  return [{ epic: null, items }];
}

export function buildGroups(batchWork: BatchWorkState | null, showCompleted: boolean): WorkItemGroup[] {
  if (!batchWork) return [];
  const active = [...batchWork.waiting, ...batchWork.inProgress];
  const all = showCompleted ? [...active, ...batchWork.done] : active;
  const nonEpics = all.filter((item) => item.kind !== "epic");
  const epics = batchWork.epics.filter((epic) => showCompleted || epic.status !== "done");

  const epicMap = new Map<string, WorkItemGroup>();
  for (const epic of epics) {
    epicMap.set(epic.id, { epic, items: [] });
  }
  const rootGroup: WorkItemGroup = { epic: null, items: [] };

  for (const item of nonEpics) {
    const parentGroup = item.parent_id ? epicMap.get(item.parent_id) : undefined;
    if (parentGroup) {
      parentGroup.items.push(item);
    } else {
      rootGroup.items.push(item);
    }
  }

  const groups: WorkItemGroup[] = [];
  if (rootGroup.items.length > 0) groups.push(rootGroup);
  const epicGroups = [...epicMap.values()].sort(
    (a, b) => (a.epic?.sort_index ?? 0) - (b.epic?.sort_index ?? 0),
  );
  for (const group of epicGroups) {
    group.items.sort((a, b) => a.sort_index - b.sort_index);
    groups.push(group);
  }
  rootGroup.items.sort((a, b) => a.sort_index - b.sort_index);
  return groups;
}

export function statusIcon(status: WorkItemStatus): string {
  switch (status) {
    case "waiting": return "○";
    case "ready": return "◔";
    case "in_progress": return "◐";
    case "to_check": return "?";
    case "blocked": return "⊘";
    case "done": return "✓";
    case "canceled": return "✕";
  }
}

export function priorityIcon(priority: WorkItemPriority): string {
  switch (priority) {
    case "urgent": return "!!";
    case "high": return "▲";
    case "medium": return "●";
    case "low": return "▽";
  }
}

export function priorityStyle(priority: WorkItemPriority): CSSProperties {
  switch (priority) {
    case "urgent": return { color: "#e06c75", fontWeight: 700 };
    case "high": return { color: "#e5a06a" };
    case "medium": return { color: "var(--muted)" };
    case "low": return { color: "var(--muted)", opacity: 0.6 };
  }
}

export const inputStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "inherit", font: "inherit", padding: "4px 6px", fontSize: 12,
};

export const miniButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "inherit", cursor: "pointer", font: "inherit", padding: "3px 6px", fontSize: 11,
};

export const deleteButtonStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "#e06c75", cursor: "pointer", font: "inherit", padding: "2px 8px", fontSize: 11,
};

export const groupHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  padding: "6px 8px",
  background: "var(--bg-2)",
  borderTop: "1px solid var(--border)",
  borderBottom: "1px solid var(--border)",
  fontSize: 11,
  textTransform: "uppercase",
  letterSpacing: 0.4,
  color: "var(--muted)",
};

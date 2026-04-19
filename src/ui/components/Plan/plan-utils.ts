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

export type WorkItemSectionKind = "inProgress" | "toDo" | "humanCheck" | "done";

export interface WorkItemSection {
  kind: WorkItemSectionKind;
  label: string;
  items: WorkItem[];
}

// Fixed top-to-bottom order and labels. Always iterated in this order by the
// renderer; empty sections are skipped there.
const SECTION_ORDER: Array<{ kind: WorkItemSectionKind; label: string }> = [
  { kind: "inProgress", label: "In progress" },
  { kind: "toDo", label: "To do" },
  { kind: "humanCheck", label: "Human check" },
  { kind: "done", label: "Done" },
];

export function classifyWorkItem(status: WorkItemStatus): WorkItemSectionKind {
  switch (status) {
    case "in_progress": return "inProgress";
    case "waiting": case "ready": case "blocked": return "toDo";
    case "human_check": return "humanCheck";
    case "done": case "canceled": return "done";
  }
}

// Default landing status when a work item is dragged *into* a section. Returns
// null for inProgress: the agent owns that status, and in-progress items are
// drag-locked anyway, so we don't let users promote items into it by drop.
export function sectionDefaultStatus(section: WorkItemSectionKind): WorkItemStatus | null {
  switch (section) {
    case "inProgress": return null;
    case "toDo": return "ready";
    case "humanCheck": return "human_check";
    case "done": return "done";
  }
}

export function splitIntoSections(items: WorkItem[]): WorkItemSection[] {
  const buckets: Record<WorkItemSectionKind, WorkItem[]> = {
    inProgress: [], toDo: [], humanCheck: [], done: [],
  };
  for (const item of items) buckets[classifyWorkItem(item.status)].push(item);
  const sections: WorkItemSection[] = [];
  for (const { kind, label } of SECTION_ORDER) {
    if (buckets[kind].length === 0) continue;
    buckets[kind].sort((a, b) => a.sort_index - b.sort_index);
    sections.push({ kind, label, items: buckets[kind] });
  }
  return sections;
}

export function buildBacklogGroups(state: BacklogState | null): WorkItemGroup[] {
  if (!state) return [];
  const items = [...state.waiting, ...state.inProgress, ...state.done];
  if (items.length === 0) return [];
  items.sort((a, b) => a.sort_index - b.sort_index);
  return [{ epic: null, items }];
}

export function buildGroups(batchWork: BatchWorkState | null): WorkItemGroup[] {
  if (!batchWork) return [];
  const all = [...batchWork.waiting, ...batchWork.inProgress, ...batchWork.done];
  const nonEpics = all.filter((item) => item.kind !== "epic");

  const epicMap = new Map<string, WorkItemGroup>();
  for (const epic of batchWork.epics) {
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
    case "human_check": return "?";
    case "blocked": return "⊘";
    case "done": return "✓";
    case "canceled": return "✕";
  }
}

export function priorityIcon(priority: WorkItemPriority): string {
  // Retained for non-visual callers (tooltips, MCP descriptions). The Plan
  // pane now renders <PriorityIcon /> (plan-icons.tsx) instead of a glyph.
  switch (priority) {
    case "urgent": return "!!";
    case "high": return "▲";
    case "medium": return "●";
    case "low": return "▽";
  }
}

/**
 * Used by callers that still render the priority as a coloured glyph
 * (context menus, etc.). The Plan pane now prefers <PriorityIcon /> so the
 * three bars render at a fixed pixel width — see plan-icons.tsx.
 */
export function priorityStyle(priority: WorkItemPriority): CSSProperties {
  switch (priority) {
    case "urgent": return { color: "var(--priority-urgent)", fontWeight: 700 };
    case "high": return { color: "var(--priority-high)" };
    case "medium": return { color: "var(--priority-medium)" };
    case "low": return { color: "var(--priority-low)" };
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

export const sectionHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  padding: "4px 10px",
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.6,
  color: "var(--muted)",
  borderTop: "1px solid var(--border)",
  background: "var(--bg)",
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

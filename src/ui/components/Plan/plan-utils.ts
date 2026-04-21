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
  epicChildren: Map<string, WorkItem[]>;
}

export type WorkItemSectionKind = "inProgress" | "toDo" | "humanCheck" | "blocked" | "done";

export interface WorkItemSection {
  kind: WorkItemSectionKind;
  label: string;
  items: WorkItem[];
}

// Fixed top-to-bottom order and labels. Always iterated in this order by the
// renderer; empty sections are skipped there.
const SECTION_ORDER: Array<{ kind: WorkItemSectionKind; label: string }> = [
  { kind: "inProgress", label: "In progress" },
  { kind: "toDo", label: "To Do" },
  { kind: "humanCheck", label: "Human check" },
  { kind: "blocked", label: "Blocked" },
  { kind: "done", label: "Done" },
];

export function classifyWorkItem(status: WorkItemStatus): WorkItemSectionKind {
  switch (status) {
    case "in_progress": return "inProgress";
    case "ready": return "toDo";
    case "blocked": return "blocked";
    case "human_check": return "humanCheck";
    // `archived` rolls into the Done section — the done-section header
    // owns a "Show archived" toggle that controls whether those rows are
    // visible. Keeping archived in its own section cluttered the panel.
    case "done": case "canceled": case "archived": return "done";
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
    case "blocked": return "blocked";
    case "done": return "done";
  }
}

export function splitIntoSections(items: WorkItem[]): WorkItemSection[] {
  const buckets: Record<WorkItemSectionKind, WorkItem[]> = {
    inProgress: [], toDo: [], humanCheck: [], blocked: [], done: [],
  };
  for (const item of items) buckets[classifyWorkItem(item.status)].push(item);
  const sections: WorkItemSection[] = [];
  for (const { kind, label } of SECTION_ORDER) {
    if (buckets[kind].length === 0) continue;
    buckets[kind].sort((a, b) =>
      kind === "humanCheck" ? b.sort_index - a.sort_index : a.sort_index - b.sort_index
    );
    sections.push({ kind, label, items: buckets[kind] });
  }
  return sections;
}

export function buildBacklogGroups(state: BacklogState | null): WorkItemGroup[] {
  if (!state) return [];
  const items = [...state.waiting, ...state.inProgress, ...state.done];
  if (items.length === 0) return [];
  items.sort((a, b) => a.sort_index - b.sort_index);
  return [{ epic: null, items, epicChildren: new Map() }];
}

export function buildGroups(batchWork: BatchWorkState | null): WorkItemGroup[] {
  if (!batchWork) return [];
  const all = [...batchWork.waiting, ...batchWork.inProgress, ...batchWork.done];

  const epicChildrenMap = new Map<string, WorkItem[]>();
  const epicIdSet = new Set(batchWork.epics.map((e) => e.id));

  for (const epic of batchWork.epics) {
    epicChildrenMap.set(epic.id, []);
  }

  const rootItems: WorkItem[] = [];
  for (const item of all) {
    if (item.kind === "epic") continue;
    if (item.parent_id && epicIdSet.has(item.parent_id)) {
      epicChildrenMap.get(item.parent_id)!.push(item);
      // in_progress children also surface in the top-level In Progress section
      // so they're visible without expanding the epic. All other statuses stay
      // exclusively inside the epic pane.
      if (item.status === "in_progress") rootItems.push(item);
    } else {
      rootItems.push(item);
    }
  }

  for (const children of epicChildrenMap.values()) {
    children.sort((a, b) => a.sort_index - b.sort_index);
  }

  const epicsAndRoots: WorkItem[] = [
    ...batchWork.epics,
    ...rootItems,
  ].sort((a, b) => a.sort_index - b.sort_index);

  return [{ epic: null, items: epicsAndRoots, epicChildren: epicChildrenMap }];
}

// User-facing label for a status. The raw id ("human_check", "in_progress")
// still flows through the wire and the `value` on <select> options, but every
// label the user sees goes through this helper so tweaks land in one place.
export function statusLabel(status: WorkItemStatus): string {
  switch (status) {
    case "ready": return "To Do";
    case "in_progress": return "In Progress";
    case "human_check": return "Human Check";
    case "blocked": return "Blocked";
    case "done": return "Done";
    case "canceled": return "Canceled";
    case "archived": return "Archived";
  }
}

export function statusIcon(status: WorkItemStatus): string {
  switch (status) {
    case "ready": return "○";
    case "in_progress": return "◐";
    case "human_check": return "?";
    case "blocked": return "⊘";
    case "done": return "✓";
    case "canceled": return "✕";
    case "archived": return "▣";
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

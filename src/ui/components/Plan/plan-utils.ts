import type { CSSProperties } from "react";
import { useState } from "react";
import type {
  BacklogState,
  ThreadWorkState,
  WorkItem,
  WorkItemPriority,
  WorkItemStatus,
} from "../../api.js";

// Keys for collapsible sections in the Plan pane. Extends
// WorkItemSectionKind with the pseudo-sections that PlanPane injects
// alongside the work-item sections (e.g. Recent answers). All share
// a single collapsed-state Set so toggling works consistently.
export type PlanSectionKey = WorkItemSectionKind | "recentAnswers";

/**
 * Hook: manages a Set of collapsed section keys, persisted to
 * localStorage under `oxplow.plan.collapsed`. Shared by WorkGroupList
 * (for work-item sections) and RecentAnswersList (for its own pseudo-
 * section) so every collapsible section in the Plan pane uses one
 * source of truth.
 */
export function useCollapsedSections(): {
  collapsed: Set<PlanSectionKey>;
  toggle: (kind: PlanSectionKey) => void;
  isCollapsed: (kind: PlanSectionKey) => boolean;
} {
  const [collapsed, setCollapsed] = useState<Set<PlanSectionKey>>(() => {
    try {
      const raw = typeof window !== "undefined" ? window.localStorage?.getItem("oxplow.plan.collapsed") : null;
      if (!raw) return new Set();
      const parsed = JSON.parse(raw);
      return new Set(Array.isArray(parsed) ? parsed as PlanSectionKey[] : []);
    } catch { return new Set(); }
  });
  const toggle = (kind: PlanSectionKey) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(kind)) next.delete(kind); else next.add(kind);
      try { window.localStorage?.setItem("oxplow.plan.collapsed", JSON.stringify([...next])); } catch { /* noop */ }
      return next;
    });
  };
  const isCollapsed = (kind: PlanSectionKey) => collapsed.has(kind);
  return { collapsed, toggle, isCollapsed };
}

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
  { kind: "blocked", label: "Blocked" },
  { kind: "humanCheck", label: "Human check" },
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

/**
 * Effective section for an epic, derived from its children's statuses.
 * Epics move between sections as a block — the epic + all its children
 * render together under whichever section the rollup picks. Children
 * retain their literal statuses (badges, drag rules, etc.); only the
 * epic's *placement* changes.
 *
 * Priority order:
 *   1. any child blocked → blocked
 *   2. all children terminal (done/canceled/archived) → done
 *   3. all children human_check → humanCheck
 *   4. any child in_progress, or any done child mixed with non-done
 *      non-blocked siblings → inProgress
 *   5. all children ready → toDo
 *
 * Edge cases: an epic with no children falls back to its own literal
 * status; an empty epic that's `ready` goes to To Do, etc.
 */
export function classifyEpic(epic: WorkItem, children: WorkItem[]): WorkItemSectionKind {
  if (children.length === 0) return classifyWorkItem(epic.status);
  let anyBlocked = false;
  let anyInProgress = false;
  let anyDone = false;
  let allTerminal = true;
  let allHumanCheck = true;
  let allReady = true;
  for (const child of children) {
    const s = child.status;
    if (s === "blocked") anyBlocked = true;
    if (s === "in_progress") anyInProgress = true;
    if (s === "done" || s === "canceled" || s === "archived") anyDone = true;
    if (s !== "done" && s !== "canceled" && s !== "archived") allTerminal = false;
    if (s !== "human_check") allHumanCheck = false;
    if (s !== "ready") allReady = false;
  }
  if (anyBlocked) return "blocked";
  if (allTerminal) return "done";
  if (allHumanCheck) return "humanCheck";
  if (anyInProgress || anyDone) return "inProgress";
  if (allReady) return "toDo";
  // Fallback: mixed ready/human_check with no started or done items —
  // treat as in-progress so review-pending work doesn't hide in To Do.
  return "inProgress";
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

/**
 * Classify a row for section placement, applying epic rollup when the
 * row is an epic. Non-epics use their literal status. Pass the
 * epicChildrenMap from `buildGroups` so the rollup sees the same
 * children the renderer will display.
 */
export function classifyRow(
  item: WorkItem,
  epicChildrenMap: Map<string, WorkItem[]>,
): WorkItemSectionKind {
  if (item.kind === "epic") {
    return classifyEpic(item, epicChildrenMap.get(item.id) ?? []);
  }
  return classifyWorkItem(item.status);
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
      (kind === "humanCheck" || kind === "done") ? b.sort_index - a.sort_index : a.sort_index - b.sort_index
    );
    sections.push({ kind, label, items: buckets[kind] });
  }
  return sections;
}

/**
 * The Human Check and Done sections render descending (newest / highest
 * sort_index on top) so recent items stay visible without scrolling. Every
 * other section renders ascending. Persistence is a single ascending
 * sort_index space per thread, so when we flatten the visual order into an id
 * list for the store we need to flip descending runs back to ascending —
 * otherwise the store's "rewrite sort_index = position" rule would invert
 * them on the next render and drag-reorders inside the section would
 * visually jump in the opposite direction.
 *
 * This helper takes a flat list of rows in **visual** order and returns the
 * list of ids in **persistence** order. Rows outside descending runs are
 * kept in place; descending runs are reversed in situ.
 */
const DESCENDING_STATUSES: ReadonlySet<WorkItemStatus> = new Set([
  "human_check",
  "done",
  "canceled",
  "archived",
]);

export function finalizeReorderIds(
  rows: ReadonlyArray<{ id: string; status: WorkItemStatus }>,
): string[] {
  const ids = rows.map((row) => row.id);
  let runStart = -1;
  const flipRun = (end: number) => {
    if (runStart < 0) return;
    // Reverse ids between runStart (inclusive) and end (exclusive).
    let lo = runStart;
    let hi = end - 1;
    while (lo < hi) {
      const tmp = ids[lo]!;
      ids[lo] = ids[hi]!;
      ids[hi] = tmp;
      lo++; hi--;
    }
    runStart = -1;
  };
  for (let i = 0; i < rows.length; i++) {
    const status = rows[i]!.status;
    const inDescRun = DESCENDING_STATUSES.has(status);
    // Group human_check separately from done so a boundary between them flips.
    // The done section (done/canceled/archived) is one run; human_check is its own.
    const runKind = status === "human_check" ? "hc" : (inDescRun ? "done" : null);
    const prevKind = runStart >= 0 ? (rows[runStart]!.status === "human_check" ? "hc" : "done") : null;
    if (runKind && runKind === prevKind) {
      // continue current run
    } else if (runKind) {
      flipRun(i);
      runStart = i;
    } else {
      flipRun(i);
    }
  }
  flipRun(rows.length);
  return ids;
}

export function buildBacklogGroups(state: BacklogState | null): WorkItemGroup[] {
  // Always yield exactly one root group, even when the backlog is empty or
  // `state` is still loading — the Plan pane renders the section chrome
  // (To Do / Done / etc. + the "⋯ New task" menu) through WorkGroupList,
  // which only runs when a group exists. Without a group the empty backlog
  // would fall back to a blank "Backlog is empty." label with no way to
  // create the first task.
  const items = state ? [...state.waiting, ...state.inProgress, ...state.done] : [];
  items.sort((a, b) => a.sort_index - b.sort_index);
  return [{ epic: null, items, epicChildren: new Map() }];
}

export function buildGroups(threadWork: ThreadWorkState | null): WorkItemGroup[] {
  if (!threadWork) return [];
  const all = [...threadWork.waiting, ...threadWork.inProgress, ...threadWork.done];

  const epicChildrenMap = new Map<string, WorkItem[]>();
  const epicIdSet = new Set(threadWork.epics.map((e) => e.id));

  for (const epic of threadWork.epics) {
    epicChildrenMap.set(epic.id, []);
  }

  const rootItems: WorkItem[] = [];
  for (const item of all) {
    if (item.kind === "epic") continue;
    if (item.parent_id && epicIdSet.has(item.parent_id)) {
      epicChildrenMap.get(item.parent_id)!.push(item);
      // Children render exclusively inside the epic pane; the epic
      // itself moves between sections as a block based on its rollup
      // (`classifyEpic`), bringing its expand toggle + child rows with
      // it. No surface-to-root lift.
    } else {
      rootItems.push(item);
    }
  }

  for (const children of epicChildrenMap.values()) {
    children.sort((a, b) => a.sort_index - b.sort_index);
  }

  const epicsAndRoots: WorkItem[] = [
    ...threadWork.epics,
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
  padding: "6px 10px",
  fontSize: 11,
  fontWeight: 600,
  textTransform: "uppercase",
  letterSpacing: 0.8,
  color: "var(--fg)",
  borderTop: "1px solid var(--border-strong, var(--border))",
  borderBottom: "1px solid var(--border)",
  background: "var(--bg-2)",
  position: "sticky",
  top: 0,
  zIndex: 1,
};

// Action-button style shared by every section header. Promoted from
// WorkGroupList's previous private `miniDoneHeaderButtonStyle` so every
// section's action buttons read as one family. Compact enough for icon-
// only buttons (+ New Task, + Commit Point, etc.) without crowding the
// section header — the header is narrow in practice.
export const sectionActionButtonStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "var(--fg)",
  cursor: "pointer",
  font: "inherit",
  padding: "2px 6px",
  fontSize: 12,
  lineHeight: 1,
  minWidth: 22,
  textAlign: "center",
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

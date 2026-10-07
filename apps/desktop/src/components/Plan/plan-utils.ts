import type { CSSProperties } from "react";
import { useState } from "react";
import { STATE_LABEL, type CanonicalState, type WorkItem, type WorkList } from "../../workItems.js";

// Keys for collapsible sections in the Plan pane. Extends
// TaskSectionKind with the pseudo-sections that PlanPane injects
// alongside the tasks sections (e.g. Recent answers). All share
// a single collapsed-state Set so toggling works consistently.
export type PlanSectionKey = TaskSectionKind | "recentAnswers";

/**
 * Hook: manages a Set of collapsed section keys, persisted to
 * localStorage under `oxplow.plan.collapsed`. Shared by TaskGroupList
 * (for tasks sections) and RecentAnswersList (for its own pseudo-
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

export interface TaskGroup {
  epic: WorkItem | null;
  items: WorkItem[];
  /** Each epic's children (by its ref), in list order. */
  epicChildren: Map<string, WorkItem[]>;
}

export type TaskSectionKind = "inProgress" | "ready" | "blocked" | "done";

export interface TaskSection {
  kind: TaskSectionKind;
  label: string;
  items: WorkItem[];
}

// Fixed top-to-bottom order and labels. Always iterated in this order by the
// renderer; empty sections are skipped there.
const SECTION_ORDER: Array<{ kind: TaskSectionKind; label: string }> = [
  { kind: "inProgress", label: "In progress" },
  { kind: "ready", label: "Ready" },
  { kind: "blocked", label: "Blocked" },
  { kind: "done", label: "Done" },
];

/** The section an item's state puts it in; done and canceled share Done. */
export function classifyState(state: CanonicalState): TaskSectionKind {
  switch (state) {
    case "in_progress": return "inProgress";
    case "todo": return "ready";
    case "blocked": return "blocked";
    case "done": case "canceled": return "done";
  }
}

const closed = (state: CanonicalState) => state === "done" || state === "canceled";

/**
 * Effective section for an epic, derived from its children's states.
 * Epics move between sections as a block — the epic + all its children
 * render together under whichever section the rollup picks. Children keep
 * their own states; only the epic's *placement* changes.
 *
 *   1. any child blocked → blocked
 *   2. every child closed (done / canceled) → done
 *   3. any child started, or a closed child beside open ones → inProgress
 *   4. every child ready → ready
 *
 * An epic with no children is its own state.
 */
export function classifyEpic(epic: WorkItem, children: WorkItem[]): TaskSectionKind {
  if (children.length === 0) return classifyState(epic.state);
  if (children.some((c) => c.state === "blocked")) return "blocked";
  if (children.every((c) => closed(c.state))) return "done";
  if (children.some((c) => c.state === "in_progress" || closed(c.state))) return "inProgress";
  return "ready";
}

// The state an item takes when dragged *into* a section. Null for
// inProgress: the agent owns that state, and in-progress items are
// drag-locked anyway, so a person doesn't promote items into it by drop.
export function sectionDefaultState(section: TaskSectionKind): CanonicalState | null {
  switch (section) {
    case "inProgress": return null;
    case "ready": return "todo";
    case "blocked": return "blocked";
    case "done": return "done";
  }
}

/**
 * A row's section, with the epic rollup when the row is an epic. Pass the
 * epicChildrenMap from `buildGroups` so the rollup sees the same children
 * the renderer will display.
 */
export function classifyRow(item: WorkItem, epicChildrenMap: Map<string, WorkItem[]>): TaskSectionKind {
  const children = epicChildrenMap.get(item.ref);
  if (children && children.length > 0) return classifyEpic(item, children);
  return classifyState(item.state);
}

/** Items (in list order) by section; Done renders newest first. */
export function splitIntoSections(items: WorkItem[]): TaskSection[] {
  const buckets: Record<TaskSectionKind, WorkItem[]> = { inProgress: [], ready: [], blocked: [], done: [] };
  for (const item of items) buckets[classifyState(item.state)].push(item);
  const sections: TaskSection[] = [];
  for (const { kind, label } of SECTION_ORDER) {
    if (buckets[kind].length === 0) continue;
    sections.push({ kind, label, items: kind === "done" ? [...buckets[kind]].reverse() : buckets[kind] });
  }
  return sections;
}

/**
 * The Done section renders descending (the latest on its list on top) so
 * recent items stay visible without scrolling; every other section
 * ascending. A list has one order, so a drag's visual order is flattened
 * back to list order before it's sent: descending runs are reversed in
 * situ, the rest kept.
 */
export function finalizeReorderRefs(rows: ReadonlyArray<{ ref: string; state: CanonicalState }>): string[] {
  const refs = rows.map((row) => row.ref);
  let runStart = -1;
  const flipRun = (end: number) => {
    if (runStart < 0) return;
    let lo = runStart;
    let hi = end - 1;
    while (lo < hi) {
      const tmp = refs[lo]!;
      refs[lo] = refs[hi]!;
      refs[hi] = tmp;
      lo++;
      hi--;
    }
    runStart = -1;
  };
  for (let i = 0; i < rows.length; i++) {
    const inDescRun = closed(rows[i]!.state);
    if (inDescRun && runStart < 0) runStart = i;
    else if (!inDescRun) flipRun(i);
  }
  flipRun(rows.length);
  return refs;
}

/** Outstanding backlog work for its badge: everything not closed. */
export function openBacklogCount(backlog: WorkList | null): number {
  if (!backlog) return 0;
  return backlog.items.length + backlog.waiting.length + backlog.inProgress.length;
}

/**
 * The backlog as one group. Always exactly one, even empty or loading:
 * the Plan renders its section chrome (and "New item") through a group.
 */
export function buildBacklogGroups(backlog: WorkList | null): TaskGroup[] {
  return [{ epic: null, items: backlog ? [...backlog.all] : [], epicChildren: new Map() }];
}

/** The chosen values per declared field (`{ priority: ["high"] }`); a
 *  field with none chosen doesn't filter. */
export type FieldFilter = Record<string, readonly string[]>;

/**
 * Keep the rows whose own fields match every chosen value. An epic row
 * stays whatever its own value — it anchors its children, which filter
 * the same way.
 */
export function filterByFields(groups: TaskGroup[], filter: FieldFilter): TaskGroup[] {
  const active = Object.entries(filter).filter(([, values]) => values.length > 0);
  if (active.length === 0) return groups;
  const keep = (item: WorkItem) =>
    active.every(([name, values]) => {
      const v = item.native[name];
      return v !== undefined && v !== null && values.includes(String(v));
    });
  return groups.map((group) => {
    const isParent = (ref: string) => (group.epicChildren.get(ref)?.length ?? 0) > 0;
    const epicChildren = new Map<string, WorkItem[]>();
    for (const [epic, children] of group.epicChildren.entries()) epicChildren.set(epic, children.filter(keep));
    return { epic: group.epic, items: group.items.filter((i) => isParent(i.ref) || keep(i)), epicChildren };
  });
}

/** Keep or drop rows by state (an epic's children too). */
export function applyStateFilter(
  groups: TaskGroup[],
  opts: { only?: CanonicalState[]; exclude?: CanonicalState[] },
): TaskGroup[] {
  const keep = (item: WorkItem) =>
    (!opts.only || opts.only.includes(item.state)) && !(opts.exclude && opts.exclude.includes(item.state));
  return groups.map((group) => {
    const epicChildren = new Map<string, WorkItem[]>();
    for (const [epic, children] of group.epicChildren.entries()) epicChildren.set(epic, children.filter(keep));
    return { epic: group.epic, items: group.items.filter(keep), epicChildren };
  });
}

/** A thread's list as one group: epics with their children under them,
 *  every row in list order. */
export function buildGroups(list: WorkList | null): TaskGroup[] {
  if (!list) return [];
  const epics = new Set(list.epics.map((e) => e.ref));
  const epicChildren = new Map<string, WorkItem[]>();
  for (const e of list.epics) epicChildren.set(e.ref, []);
  const roots: WorkItem[] = [];
  for (const item of list.all) {
    // Children render only inside their epic, which moves between sections
    // as a block (`classifyEpic`).
    if (item.parentRef && epics.has(item.parentRef)) epicChildren.get(item.parentRef)!.push(item);
    else roots.push(item);
  }
  return [{ epic: null, items: roots, epicChildren }];
}

// How a person reads a state: every label goes through this helper.
export function statusLabel(state: CanonicalState): string {
  return STATE_LABEL[state];
}

export function statusIcon(state: CanonicalState): string {
  switch (state) {
    case "todo": return "○";
    case "in_progress": return "◐";
    case "blocked": return "⊘";
    case "done": return "✓";
    case "canceled": return "✕";
  }
}

export const inputStyle: CSSProperties = {
  borderRadius: 6, border: "1px solid var(--border)", background: "var(--bg)", color: "inherit", font: "inherit", padding: "4px 6px", fontSize: "var(--text-xs)",
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
  padding: "10px 12px",
  fontSize: 11,
  fontWeight: 600,
  textTransform: "uppercase",
  letterSpacing: 0.8,
  color: "var(--text-secondary)",
  borderTop: "1px solid var(--border-subtle)",
  borderBottom: "1px solid var(--border-subtle)",
  background: "var(--surface-app)",
  position: "sticky",
  top: 0,
  zIndex: 1,
};

// Action-button style shared by every section header. Promoted from
// TaskGroupList's previous private `miniDoneHeaderButtonStyle` so every
// section's action buttons read as one family. Compact enough for icon-
// only buttons without crowding the section header.
export const sectionActionButtonStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "var(--fg)",
  cursor: "pointer",
  font: "inherit",
  padding: "2px 6px",
  fontSize: "var(--text-xs)",
  lineHeight: 1,
  minWidth: 22,
  textAlign: "center",
};

export const groupHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  padding: "10px 12px",
  background: "var(--surface-app)",
  borderTop: "1px solid var(--border-subtle)",
  borderBottom: "1px solid var(--border-subtle)",
  fontSize: 11,
  textTransform: "uppercase",
  letterSpacing: 0.4,
  color: "var(--text-secondary)",
};

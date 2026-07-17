import type { ThreadWorkState, Task } from "../../api.js";
import type { TabRef } from "../../tabs/tabState.js";
import {
  archivedRef,
  backlogRef,
  changeAnalysisRef,
  closedThreadsRef,
  dashboardRef,
  doneWorkRef,
  gitDashboardRef,
  indexRef,
  tasksRef,
  uncommittedChangesRef,
} from "../../tabs/pageRefs.js";

/**
 * Sections the launcher's "start menu" empty state groups pages under.
 * `PAGE_CATEGORY_ORDER` is the render order for those headings.
 */
export type PageCategory = "Work" | "Code" | "Git" | "Activity" | "Knowledge" | "System";

export const PAGE_CATEGORY_ORDER: readonly PageCategory[] = [
  "Work",
  "Code",
  "Git",
  "Activity",
  "Knowledge",
  "System",
];

export interface PageDirectoryEntry {
  id: string;
  label: string;
  ref: TabRef;
  category: PageCategory;
  badge?: number;
  /** Extra search terms the launcher fuzzy-matches beyond label/id — for
   *  a page whose name doesn't contain a word users reach for (e.g. the
   *  "Tasks" page found by typing "dashboard"). */
  keywords?: string;
}

/**
 * Static directory of every top-level page. This is the single discovery
 * surface: the launcher (QuickOpen) shows it grouped by `category` in its
 * empty state and mixes it into ranked results when the user types. The
 * rail no longer renders a "Pages" section — users pin what they want via
 * Bookmarks instead. Entries are listed grouped by category so the flat
 * launcher order already reads top-to-bottom by section. Pure helper so it
 * can be unit-tested without mounting React. `backlogReadyCount` controls
 * the badge on "Backlog".
 */
export function computePagesDirectory(opts: { backlogReadyCount: number }): PageDirectoryEntry[] {
  return [
    // Labels are emoji-free — `PageKindIcon` resolves the leading
    // glyph from the entry's ref kind at render time.
    { id: "tasks", label: "Tasks", ref: tasksRef(), category: "Work", keywords: "dashboard" },
    { id: "done-work", label: "Done Work", ref: doneWorkRef(), category: "Work" },
    {
      id: "backlog",
      label: "Backlog",
      ref: backlogRef(),
      category: "Work",
      badge: opts.backlogReadyCount > 0 ? opts.backlogReadyCount : undefined,
    },
    { id: "archived", label: "Archived", ref: archivedRef(), category: "Work" },
    { id: "files", label: "Files", ref: indexRef("files"), category: "Code" },
    { id: "git-dashboard", label: "Git", ref: gitDashboardRef(), category: "Git" },
    { id: "git-history", label: "Git History", ref: indexRef("git-history"), category: "Git" },
    { id: "uncommitted-changes", label: "Uncommitted", ref: uncommittedChangesRef(), category: "Git" },
    { id: "change-analysis", label: "Change Analysis", ref: changeAnalysisRef("working"), category: "Git" },
    { id: "local-history", label: "Local History", ref: indexRef("local-history"), category: "Activity" },
    { id: "hook-events", label: "Hook Events", ref: indexRef("hook-events"), category: "Activity" },
    { id: "comments", label: "Comments Dashboard", ref: indexRef("comments"), category: "Activity" },
    { id: "dashboard-planning", label: "Planning", ref: dashboardRef("planning"), category: "Activity" },
    { id: "dashboard-review", label: "Review", ref: dashboardRef("review"), category: "Activity" },
    { id: "dashboard-quality", label: "Quality", ref: dashboardRef("quality"), category: "Activity" },
    { id: "dashboard-visits", label: "Go To", ref: dashboardRef("visits"), category: "Activity" },
    { id: "usage", label: "Usage", ref: indexRef("usage"), category: "Activity" },
    { id: "metrics", label: "Metrics", ref: indexRef("metrics"), category: "Activity" },
    { id: "metrics-recorded", label: "Recorded Metrics", ref: indexRef("metrics-recorded"), category: "Activity" },
    { id: "wiki-index", label: "Wiki", ref: indexRef("wiki-index"), category: "Knowledge" },
    { id: "terminal", label: "Terminal", ref: indexRef("terminal"), category: "System" },
    { id: "closed-threads", label: "Closed Threads", ref: closedThreadsRef(), category: "System" },
    { id: "settings", label: "Settings", ref: indexRef("settings"), category: "System" },
  ];
}

/**
 * Pick the lowest-sort_index `in_progress` non-epic item from a thread's
 * work state. The "Active item" rail section anchors on this.
 *
 * The store's `inProgress` bucket holds `in_progress` items. The rail's
 * "Active item" means *what the agent is doing right now*.
 */
export function computeActiveItem(state: ThreadWorkState | null): Task | null {
  if (!state) return null;
  const epicIds = new Set(state.epics.map((e) => e.id));
  const candidates = state.inProgress.filter(
    (item) => item.status === "in_progress" && !epicIds.has(item.id),
  );
  if (candidates.length === 0) return null;
  return candidates.reduce((best, current) =>
    current.sort_index < best.sort_index ? current : best,
  );
}

/**
 * If the active in-progress item is a child of an epic, return the epic
 * and its non-archived children (sorted by sort_index ascending). When
 * the active item is standalone, returns null.
 */
export function computeActiveEpicContext(
  state: ThreadWorkState | null,
  active: Task | null,
): { epic: Task; children: Task[] } | null {
  if (!state || !active || !active.parent_id) return null;
  // Only treat the parent as an "epic" if it is in state.epics — i.e. it
  // has children (the runtime classifies any task with children as an
  // epic). A plain task whose id happens to match active.parent_id is
  // not an epic anchor.
  const epic = state.epics.find((i) => i.id === active.parent_id);
  if (!epic) return null;
  const pool = state.items.length > 0 ? state.items : [...state.epics, ...state.inProgress, ...state.waiting, ...state.done];
  const children = pool
    .filter((i) => i.parent_id === epic.id && i.status !== "archived")
    .sort((a, b) => a.sort_index - b.sort_index);
  return { epic, children };
}

/**
 * Return the next-up `ready` items, sorted by sort_index ascending,
 * truncated to `limit`. The "Ready" rail section uses this.
 */
export function computeUpNext(state: ThreadWorkState | null, limit = 5): Task[] {
  if (!state) return [];
  const ready = state.items.filter((item) => item.status === "ready");
  ready.sort((a, b) => a.sort_index - b.sort_index);
  return ready.slice(0, limit);
}

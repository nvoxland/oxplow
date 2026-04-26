import type { ThreadWorkState, WorkItem } from "../../api.js";
import type { TabRef } from "../../tabs/tabState.js";
import {
  dashboardRef,
  gitDashboardRef,
  indexRef,
  uncommittedChangesRef,
} from "../../tabs/pageRefs.js";

export interface PageDirectoryEntry {
  id: string;
  label: string;
  ref: TabRef;
  badge?: number;
}

/**
 * Static directory of top-level pages shown in the rail's "Pages" section.
 * Pure helper so it can be unit-tested without mounting the React component.
 * `backlogReadyCount` controls the badge on "All work".
 */
export function computePagesDirectory(opts: { backlogReadyCount: number }): PageDirectoryEntry[] {
  return [
    { id: "start", label: "⌂  Start", ref: indexRef("start") },
    {
      id: "all-work",
      label: "📋  All work",
      ref: indexRef("all-work"),
      badge: opts.backlogReadyCount > 0 ? opts.backlogReadyCount : undefined,
    },
    { id: "notes-index", label: "📒  Notes", ref: indexRef("notes-index") },
    { id: "files", label: "📁  Files", ref: indexRef("files") },
    { id: "code-quality", label: "⚠  Code quality", ref: indexRef("code-quality") },
    { id: "local-history", label: "⏱  Local history", ref: indexRef("local-history") },
    { id: "git-dashboard", label: "🌐  Git dashboard", ref: gitDashboardRef() },
    { id: "uncommitted-changes", label: "✎  Uncommitted", ref: uncommittedChangesRef() },
    { id: "git-history", label: "🌿  Git history", ref: indexRef("git-history") },
    { id: "hook-events", label: "🪝  Hook events", ref: indexRef("hook-events") },
    { id: "subsystem-docs", label: "📑  Subsystem docs", ref: indexRef("subsystem-docs") },
    { id: "settings", label: "⚙  Settings", ref: indexRef("settings") },
    { id: "dashboard-planning", label: "📊  Planning", ref: dashboardRef("planning") },
    { id: "dashboard-review", label: "📊  Review", ref: dashboardRef("review") },
    { id: "dashboard-quality", label: "📊  Quality", ref: dashboardRef("quality") },
  ];
}

/**
 * Pick the lowest-sort_index `in_progress` non-epic item from a thread's
 * work state. The "Active item" rail section anchors on this.
 *
 * The store's `inProgress` bucket packs both `in_progress` and `human_check`
 * (kanban-view convenience). The rail's "Active item" means *what the agent
 * is doing right now*, so human_check items — already shipped, awaiting
 * review — are excluded here.
 */
export function computeActiveItem(state: ThreadWorkState | null): WorkItem | null {
  if (!state) return null;
  const candidates = state.inProgress.filter(
    (item) => item.kind !== "epic" && item.status === "in_progress",
  );
  if (candidates.length === 0) return null;
  return candidates.reduce((best, current) =>
    current.sort_index < best.sort_index ? current : best,
  );
}

/**
 * Return the next-up `ready` items, sorted by sort_index ascending,
 * truncated to `limit`. The "Up next" rail section uses this.
 */
export function computeUpNext(state: ThreadWorkState | null, limit = 5): WorkItem[] {
  if (!state) return [];
  const ready = state.items.filter((item) => item.status === "ready" && item.kind !== "epic");
  ready.sort((a, b) => a.sort_index - b.sort_index);
  return ready.slice(0, limit);
}

export interface RecentFileEntry {
  path: string;
  touchedAt: number;
}

/** Sort recent files newest-first; truncate to `limit`. */
export function sortRecentFiles(entries: RecentFileEntry[], limit = 8): RecentFileEntry[] {
  return [...entries].sort((a, b) => b.touchedAt - a.touchedAt).slice(0, limit);
}

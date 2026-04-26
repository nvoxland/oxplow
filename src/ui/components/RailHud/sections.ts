import type { ThreadWorkState, WorkItem } from "../../api.js";

/**
 * Pick the lowest-sort_index `in_progress` non-epic item from a thread's
 * work state. The "Active item" rail section anchors on this.
 */
export function computeActiveItem(state: ThreadWorkState | null): WorkItem | null {
  if (!state) return null;
  const candidates = state.inProgress.filter((item) => item.kind !== "epic");
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

// Pure selection helpers for the tab strip's right-click "close" actions.
// Kept React-free so the before/after-anchor math is unit-tested without
// mounting CenterTabs. Both operate over the strip order (the visible,
// drag-reorderable list) and only ever return *closable* ids — pinned
// tabs (the Agent tab) are never close targets.

export interface ClosableTab {
  id: string;
  closable: boolean;
}

/** Closable tab ids other than `anchorId`, in strip order. */
export function tabsToCloseOthers(tabs: ClosableTab[], anchorId: string): string[] {
  return tabs.filter((t) => t.closable && t.id !== anchorId).map((t) => t.id);
}

/** Closable tab ids positioned after `anchorId` in strip order. */
export function tabsToCloseRight(tabs: ClosableTab[], anchorId: string): string[] {
  const idx = tabs.findIndex((t) => t.id === anchorId);
  if (idx < 0) return [];
  return tabs
    .slice(idx + 1)
    .filter((t) => t.closable)
    .map((t) => t.id);
}

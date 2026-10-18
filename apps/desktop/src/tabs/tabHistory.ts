/// A page tab's back/forward history as in-tab navigation changes it.
import type { NavSiblings } from "./PageNavigationContext.js";
import type { HistoryFrame } from "./pageTabsPersistence.js";
import type { TabRef } from "./tabState.js";

export interface TabHistory {
  back: HistoryFrame[];
  forward: HistoryFrame[];
  siblings: NavSiblings | null;
}

/**
 * Take a caller-supplied siblings record, snap its `index` to the
 * position whose `ref.id` matches the destination, and drop the
 * record entirely if there's no match (a stale list shouldn't drive
 * prev/next on a page that isn't actually in it).
 */
export function resolveSiblings(siblings: NavSiblings | undefined, ref: TabRef): NavSiblings | null {
  if (!siblings || siblings.entries.length === 0) return null;
  const matchIdx = siblings.entries.findIndex((e) => e.ref.id === ref.id);
  if (matchIdx < 0) {
    // Caller passed `siblings` but the destination isn't in the list.
    // If the supplied index points at a valid slot, trust it; otherwise
    // drop. This guards against silently rendering "1 of N" with the
    // wrong page.
    if (siblings.index < 0 || siblings.index >= siblings.entries.length) return null;
    return siblings;
  }
  return { entries: siblings.entries, index: matchIdx };
}

/** The history of a tab that navigated from `oldRef` to `ref`: the page it
 *  left goes on Back with the siblings it had (so going back restores the
 *  originating list's prev/next chain) — unless `replace`, when the page
 *  leaves history and Back skips it. Forward clears either way. */
export function navigatedHistory(
  old: TabHistory,
  oldRef: TabRef,
  ref: TabRef,
  siblings: NavSiblings | undefined,
  replace: boolean,
): TabHistory {
  return {
    back: replace ? old.back : [...old.back, { ref: oldRef, siblings: old.siblings }],
    forward: [],
    siblings: resolveSiblings(siblings, ref),
  };
}

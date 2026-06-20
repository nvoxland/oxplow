import type { NavSiblings } from "./PageNavigationContext.js";
import type { HistoryFrame } from "./pageTabsPersistence.js";
import type { TabRef } from "./tabState.js";

/** A single per-tab history record: back/forward stacks plus the
 *  siblings chain for the currently-shown page. Mirrors the value type
 *  of `ThreadHistory[threadId][tabId]`. */
export interface HistoryEntry {
  back: HistoryFrame[];
  forward: HistoryFrame[];
  siblings: NavSiblings | null;
}

export type CloseOrGoBackPlan =
  | { action: "close" }
  | { action: "back"; target: TabRef; nextEntry: HistoryEntry };

/**
 * Decide what should happen to a tab whose underlying record was just
 * deleted (e.g. a wiki page or task). If the tab has somewhere to go
 * back to, navigate there instead of closing the tab; otherwise close.
 *
 * Unlike a normal "Back" press, the deleted page is **not** pushed onto
 * the target's forward stack — forward-navigating to a record that no
 * longer exists is a dead end. The deleted page's own forward stack is
 * preserved so any pages that were ahead of it stay reachable.
 */
export function planCloseOrGoBack(
  entry: HistoryEntry | undefined,
  tabExists: boolean,
): CloseOrGoBackPlan {
  if (!tabExists || !entry || entry.back.length === 0) {
    return { action: "close" };
  }
  const targetFrame = entry.back[entry.back.length - 1]!;
  return {
    action: "back",
    target: targetFrame.ref,
    nextEntry: {
      back: entry.back.slice(0, -1),
      forward: entry.forward,
      siblings: targetFrame.siblings,
    },
  };
}

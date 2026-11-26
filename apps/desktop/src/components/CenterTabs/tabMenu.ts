// The right-click menu of a center tab, as a pure function so another shell
// (a tablet tab strip) builds the same menu. React-free: the strip passes in
// what it knows and the handlers to call.

import type { MenuItem } from "../../menu.js";
import type { WindowTab } from "../../window/windowTab.js";
import { tabsToCloseOthers, tabsToCloseRight, type ClosableTab } from "./tabClose.js";

export interface TabMenuInput {
  tab: WindowTab;
  /** The strip's open tabs in order; a pinned tab is passed as not closable. */
  bulkClosable: ClosableTab[];
  activeId: string;
  /** Without it the menu has no close entries. */
  onClose?(id: string): void;
  onActivate(id: string): void;
}

/** The tab's own entries (if any) followed by the universal Close, Close
 *  Other Tabs and Close Tabs to the Right. The close entries only appear when
 *  `onClose` is wired; the bulk ones are disabled when they have no targets.
 *  Closing the active tab with a bulk close falls the selection back to `tab`. */
export function buildTabMenu(input: TabMenuInput): MenuItem[] | Promise<MenuItem[]> {
  const { tab } = input;
  const later = tab.menuAtOpen?.() ?? [];
  if (later instanceof Promise) return later.then((atOpen) => withCloseItems(input, [...(tab.contextMenu ?? []), ...atOpen]));
  return withCloseItems(input, [...(tab.contextMenu ?? []), ...later]);
}

function withCloseItems({ tab, bulkClosable, activeId, onClose, onActivate }: TabMenuInput, items: MenuItem[]): MenuItem[] {
  if (!onClose) return items;
  // Close a batch through the single-tab close path, then, only if that swept
  // away the active tab, fall the selection back to the anchor.
  const closeMany = (ids: string[]): void => {
    if (ids.length === 0) return;
    const closingActive = ids.includes(activeId);
    for (const id of ids) onClose(id);
    if (closingActive) onActivate(tab.id);
  };
  const others = tabsToCloseOthers(bulkClosable, tab.id);
  const toRight = tabsToCloseRight(bulkClosable, tab.id);
  if (items.length > 0) {
    items.push({ id: "tab.close-sep", label: "", enabled: false, separator: true });
  }
  if (tab.closable && !tab.pinned) {
    items.push({ id: "tab.close", label: "Close", enabled: true, run: () => onClose(tab.id) });
  }
  items.push({ id: "tab.close-others", label: "Close Other Tabs", enabled: others.length > 0, run: () => closeMany(others) });
  items.push({ id: "tab.close-right", label: "Close Tabs to the Right", enabled: toRight.length > 0, run: () => closeMany(toRight) });
  return items;
}

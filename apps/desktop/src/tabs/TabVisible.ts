import { createContext, useContext } from "react";

/** Whether the tab body below is on screen. `CenterTabs` keeps every tab
 *  mounted and hides the others (`display: none`); a page run in a hidden
 *  tab is paused (the daemon does not re-run it) until the tab shows. Outside
 *  a tab it is on screen. */
export const TabVisibleContext = createContext(true);

export function useTabVisible(): boolean {
  return useContext(TabVisibleContext);
}

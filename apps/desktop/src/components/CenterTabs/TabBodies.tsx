import { useRef } from "react";

import { TabVisibleContext } from "../../tabs/TabVisible.js";
import type { WindowTab } from "../../window/windowTab.js";
import { ErrorBoundary } from "../ErrorBoundary.js";

/**
 * The bodies of a window's tabs, for every shell's tab strip: each body is
 * mounted when its tab is first shown and then kept, only the active one
 * visible, so Back and a tab switch keep a page as it was (scroll, open
 * trees, draft text). Hidden tabs (a tab's Back history) stay mounted too.
 * Bodies sit in their keys' order, not the strip's: a body React moves in
 * the DOM reloads any frame in it (a custom component's iframe), so a
 * reorder of the strip moves none. Each body is `center-tab-body-<id>`, the
 * id every shell draws (the e2e `shown()` helper reads it).
 */
export function TabBodies({ tabs, activeId }: { tabs: readonly WindowTab[]; activeId: string | null }) {
  const shownKeys = useRef(new Set<string>());
  const active = tabs.find((t) => t.id === activeId) ?? null;
  if (active) shownKeys.current.add(byKey(active));
  return (
    <>
      {[...tabs].sort((a, b) => byKey(a).localeCompare(byKey(b))).map((tab) => {
        const isActive = tab.id === active?.id;
        if (!isActive && !shownKeys.current.has(byKey(tab))) return null;
        return (
          <div
            key={byKey(tab)}
            style={{
              display: isActive ? "flex" : "none",
              flex: 1,
              minHeight: 0,
              minWidth: 0,
              flexDirection: "column",
              // A tab's own dialogs (`TabModal`) cover this body only.
              position: "relative",
            }}
            data-testid={`center-tab-body-${tab.id}`}
            aria-hidden={!isActive}
          >
            <ErrorBoundary label={tab.label}>
              <TabVisibleContext.Provider value={isActive}>{tab.render()}</TabVisibleContext.Provider>
            </ErrorBoundary>
          </div>
        );
      })}
    </>
  );
}

function byKey(tab: WindowTab): string {
  return tab.key ?? tab.id;
}

import { useSyncExternalStore } from "react";
import { createTabStore, type TabStore, type ThreadTabState } from "./tabState.js";

let singleton: TabStore | null = null;

/** Lazily create and reuse a single store across the renderer. */
export function getTabStore(): TabStore {
  if (!singleton) singleton = createTabStore();
  return singleton;
}

/** For tests: replace the singleton with a fresh store. */
export function resetTabStore(): void {
  singleton = null;
}

/** Subscribe to a thread's tab state and re-render on changes. */
export function useThreadTabs(threadId: string | null): ThreadTabState {
  const store = getTabStore();
  // useSyncExternalStore is stable across React versions and gives us
  // tear-free subscriptions in concurrent rendering.
  const subscribe = (fn: () => void) => {
    if (!threadId) return () => {};
    return store.subscribe(threadId, fn);
  };
  const getSnapshot = () =>
    threadId ? store.getThreadState(threadId) : { tabs: [], activeId: null };
  // The default getServerSnapshot is fine for renderer-only use.
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

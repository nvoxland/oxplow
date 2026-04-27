import { useSyncExternalStore } from "react";
import { createBookmarksStore, type BookmarksApi } from "./bookmarks.js";

let singleton: BookmarksApi | null = null;

function safeStorage() {
  if (typeof window !== "undefined" && window.localStorage) return window.localStorage;
  // Headless / test fallback — in-memory shim.
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => { m.set(k, v); },
    removeItem: (k: string) => { m.delete(k); },
  };
}

export function getBookmarksStore(): BookmarksApi {
  if (!singleton) singleton = createBookmarksStore(safeStorage());
  return singleton;
}

/** For tests. */
export function resetBookmarksStore(): void {
  singleton = null;
}

/** Subscribe to the bookmarks store and re-render on any change. */
export function useBookmarksStore(): BookmarksApi {
  const store = getBookmarksStore();
  // useSyncExternalStore re-renders only when the snapshot CHANGES (===).
  // The store doesn't expose a versioned snapshot, so bump a module-level
  // counter on every notify and read it back here.
  useSyncExternalStore(
    (fn) => store.subscribe(() => { storeVersion++; fn(); }),
    () => storeVersion,
    () => storeVersion,
  );
  return store;
}

let storeVersion = 0;

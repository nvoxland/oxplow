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
  useSyncExternalStore(
    (fn) => store.subscribe(fn),
    () => 0, // any stable snapshot — we only care about the trigger
    () => 0,
  );
  return store;
}

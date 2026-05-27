import { useSyncExternalStore } from "react";
import { createProseAudienceStore, type ProseAudience, type ProseAudienceApi } from "./proseAudience.js";

let singleton: ProseAudienceApi | null = null;

function safeStorage() {
  if (typeof window !== "undefined" && window.localStorage) return window.localStorage;
  // Headless / test fallback — in-memory shim.
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => {
      m.set(k, v);
    },
    removeItem: (k: string) => {
      m.delete(k);
    },
  };
}

export function getProseAudienceStore(): ProseAudienceApi {
  if (!singleton) singleton = createProseAudienceStore(safeStorage());
  return singleton;
}

/** For tests. */
export function resetProseAudienceStore(): void {
  singleton = null;
}

let storeVersion = 0;

/** Read + write the per-page audience, re-rendering on any change. */
export function useProseAudience(pageKey: string | null): {
  audience: ProseAudience;
  setAudience: (a: ProseAudience) => void;
} {
  const store = getProseAudienceStore();
  // useSyncExternalStore re-renders only when the snapshot CHANGES (===).
  // The store has no versioned snapshot, so bump a module-level counter
  // on every notify and read it back (matches useBookmarks).
  useSyncExternalStore(
    (fn) =>
      store.subscribe(() => {
        storeVersion++;
        fn();
      }),
    () => storeVersion,
    () => storeVersion,
  );
  return {
    audience: store.get(pageKey),
    setAudience: (a: ProseAudience) => store.set(pageKey, a),
  };
}

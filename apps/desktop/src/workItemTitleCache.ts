import { useEffect, useState } from "react";
import { readWorkItemsByRef } from "./workItems.js";

/**
 * Shared in-memory work-item ref → title map. The wiki/markdown renderer
 * uses this to display a work-item link with the item's real title
 * instead of its id, whichever list it's on.
 *
 * Unlike the wiki cache (which lists every page up front), items are
 * resolved **lazily per ref** via `readWorkItemsByRef`, so a wiki page
 * that links a handful of items doesn't pull the whole list. A resolved
 * miss is cached as `null` so a stale/deleted ref doesn't refetch on every
 * render. Components subscribe via `useWorkItemRef(ref)`.
 */

const titles = new Map<string, string | null>();
const inFlight = new Map<string, Promise<void>>();
const listeners = new Set<() => void>();

function notify() {
  for (const fn of listeners) {
    try { fn(); } catch { /* ignore listener errors */ }
  }
}

function fetchTitle(id: string): Promise<void> {
  const existing = inFlight.get(id);
  if (existing) return existing;
  const p = (async () => {
    try {
      const { items } = await readWorkItemsByRef([id]);
      titles.set(id, items.find((r) => r.ref === id)?.title ?? null);
      notify();
    } catch {
      // Leave it unresolved so a later render retries.
    } finally {
      inFlight.delete(id);
    }
  })();
  inFlight.set(id, p);
  return p;
}

/** Existence state of a ref target. `loading` = lookup in flight;
 *  `found` / `missing` once it resolves. The renderer shows `missing`
 *  as a broken, non-clickable link. */
export type RefStatus = "loading" | "found" | "missing";

/** Snapshot the current `{title, status}` for `id` from the cache. A
 *  cached value of `null` (a resolved miss) is `missing`; a cache without
 *  the key yet is still `loading`. */
function refSnapshot(id: string | null | undefined): { title: string | null; status: RefStatus } {
  if (!id || !titles.has(id)) return { title: null, status: "loading" };
  const title = titles.get(id) ?? null;
  return { title, status: title === null ? "missing" : "found" };
}

/**
 * Resolve a work-item ref to its title AND existence status. `status` is
 * `loading` until the lookup resolves, then `found` / `missing` (deleted
 * item, an item on a list that isn't active, a stale wikilink). Backs the
 * broken-link rendering.
 */
export function useWorkItemRef(id: string | null | undefined): { title: string | null; status: RefStatus } {
  const [state, setState] = useState(() => refSnapshot(id));

  useEffect(() => {
    if (!id) {
      setState({ title: null, status: "loading" });
      return;
    }
    const update = () => setState(refSnapshot(id));
    listeners.add(update);
    if (titles.has(id)) update();
    else void fetchTitle(id);
    return () => {
      listeners.delete(update);
    };
  }, [id]);

  return state;
}

/**
 * Resolve a work-item ref to its title. Returns `null` while the lookup is
 * in flight or if the ref isn't known — callers fall back to its label.
 */
export function useWorkItemTitle(id: string | null | undefined): string | null {
  return useWorkItemRef(id).title;
}

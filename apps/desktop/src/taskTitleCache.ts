import { useEffect, useState } from "react";
import { getTaskSummaries } from "./api.js";

/**
 * Shared in-memory task-id → title map. The wiki/markdown renderer uses
 * this to display `[[tsk42]]` wikilinks with the task's real title instead
 * of the raw id — readers should see the task title, not `tsk42`.
 *
 * Unlike the wiki cache (which lists every page up front), tasks are
 * resolved **lazily per id** via `getTaskSummaries`, so a wiki page that
 * links a handful of tasks doesn't pull the whole task list. A resolved
 * miss is cached as `null` so a stale/deleted id doesn't refetch on every
 * render. Components subscribe via `useTaskTitle(id)`.
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
      const rows = await getTaskSummaries([id]);
      titles.set(id, rows.find((r) => r.id === id)?.title ?? null);
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
function taskRefSnapshot(id: string | null | undefined): { title: string | null; status: RefStatus } {
  if (!id || !titles.has(id)) return { title: null, status: "loading" };
  const title = titles.get(id) ?? null;
  return { title, status: title === null ? "missing" : "found" };
}

/**
 * Resolve a task id to its title AND existence status. `status` is
 * `loading` until the per-id lookup resolves, then `found` / `missing`
 * (deleted task, stale wikilink). Backs the broken-link rendering.
 */
export function useTaskRef(id: string | null | undefined): { title: string | null; status: RefStatus } {
  const [state, setState] = useState(() => taskRefSnapshot(id));

  useEffect(() => {
    if (!id) {
      setState({ title: null, status: "loading" });
      return;
    }
    const update = () => setState(taskRefSnapshot(id));
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
 * Resolve a task id to its title. Returns `null` while the lookup is in
 * flight or if the id isn't known (deleted task, stale wikilink) — callers
 * fall back to the raw id in that case.
 */
export function useTaskTitle(id: string | null | undefined): string | null {
  return useTaskRef(id).title;
}

import { useEffect, useState } from "react";
import { querySql, subscribeOxplowEvents } from "./api.js";
import { extensionTitleQuery, resourceInfo, subscribe as subscribeResources } from "./resourceStore.js";
import { identityOf, typeOf } from "./uri/uri.js";
import type { Reads } from "./tauri-bridge/generated/bindings.js";
import { coalesce, NO_READS, readsChanged } from "./viz/rerun.js";

/**
 * The one title cache for a link to any resource's row: a task, a
 * knowledge page, an extension's record. A ref's title is read from its
 * resource's `resolve` dataset and title column (`v_resource`,
 * `extensionTitleQuery`), lazily per ref, so a page that links a handful of
 * rows doesn't read a whole list. A ref is re-read when a dataset its read
 * touched changes (`readsChanged`, once a burst of changes goes quiet). A
 * ref whose resource isn't known yet stays `loading` until the resources
 * load.
 */

/** `loading`: not read yet. `found` / `missing`: the dataset has / has no
 *  row for it. `untitled`: its resource names no dataset to ask. */
export type RefStatus = "loading" | "found" | "missing" | "untitled";

export interface RefTitle {
  title: string | null;
  status: RefStatus;
}

interface Entry extends RefTitle {
  reads: Reads;
}

const LOADING: RefTitle = { title: null, status: "loading" };
const entries = new Map<string, Entry>();
/** Refs asked for before their resource loaded. */
const pending = new Set<string>();
const inFlight = new Map<string, Promise<void>>();
const listeners = new Set<() => void>();

function notify() {
  for (const fn of listeners) {
    try { fn(); } catch { /* ignore listener errors */ }
  }
}

const keyOf = (ref: string) => identityOf(ref) ?? ref;

function fetchEntry(ref: string): Promise<void> {
  const key = keyOf(ref);
  const existing = inFlight.get(key);
  if (existing) return existing;
  const resource = resourceInfo(typeOf(ref) ?? "");
  if (!resource) return Promise.resolve();
  const query = extensionTitleQuery(ref, resource);
  if (!query) {
    entries.set(key, { title: null, status: "untitled", reads: NO_READS });
    notify();
    return Promise.resolve();
  }
  const p = (async () => {
    try {
      const res = await querySql(query.sql, query.params, 1);
      const cell = res.rows[0]?.[0];
      const next: Entry =
        cell == null ? { title: null, status: "missing", reads: res.reads } : { title: String(cell), status: "found", reads: res.reads };
      const prev = entries.get(key);
      entries.set(key, next);
      if (!prev || prev.title !== next.title || prev.status !== next.status) notify();
    } catch {
      // Leave it as it was so a later render retries.
    } finally {
      inFlight.delete(key);
    }
  })();
  inFlight.set(key, p);
  return p;
}

const stale = new Set<string>();
const refreshSoon = coalesce(() => {
  const refs = [...stale];
  stale.clear();
  for (const ref of refs) void fetchEntry(ref);
});

let subscribed = false;
function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  subscribeOxplowEvents((event) => {
    for (const [ref, entry] of entries) {
      if (readsChanged(event as Record<string, unknown>, entry.reads)) stale.add(ref);
    }
    if (stale.size > 0) refreshSoon.schedule();
  });
  // A resource arriving can name a dataset for refs that were `loading`.
  subscribeResources(() => {
    for (const ref of [...pending]) {
      if (!resourceInfo(typeOf(ref) ?? "")) continue;
      pending.delete(ref);
      void fetchEntry(ref);
    }
  });
}

function snapshot(ref: string | null | undefined): RefTitle {
  if (!ref) return LOADING;
  const entry = entries.get(keyOf(ref));
  return entry ? { title: entry.title, status: entry.status } : LOADING;
}

function ask(ref: string): void {
  ensureSubscribed();
  if (!entries.has(keyOf(ref))) {
    if (resourceInfo(typeOf(ref) ?? "")) void fetchEntry(ref);
    else pending.add(ref);
  }
}

/**
 * A ref's title and existence status: `loading` until read, then `found` /
 * `missing` (a deleted row, a stale link). Backs the title swap and the
 * broken-link rendering.
 */
export function useRefTitle(ref: string | null | undefined): RefTitle {
  const [state, setState] = useState(() => snapshot(ref));
  useEffect(() => {
    if (!ref) {
      setState(LOADING);
      return;
    }
    const update = () =>
      setState((prev) => {
        const next = snapshot(ref);
        return prev.title === next.title && prev.status === next.status ? prev : next;
      });
    listeners.add(update);
    ask(ref);
    update();
    return () => {
      listeners.delete(update);
    };
  }, [ref]);
  return state;
}

/** The same as `useRefTitle`, awaited (a component frame's markdown, drawn
 *  once its titles are known). */
export async function loadRefTitle(ref: string): Promise<RefTitle> {
  ensureSubscribed();
  if (!entries.has(keyOf(ref))) await fetchEntry(ref);
  return snapshot(ref);
}

/** Forget every title (a test's). */
export function clearRefTitles(): void {
  entries.clear();
  inFlight.clear();
  pending.clear();
  stale.clear();
}

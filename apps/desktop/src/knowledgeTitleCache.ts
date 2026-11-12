import { useEffect, useState } from "react";
import { subscribeOxplowEvents } from "./api.js";
import { readKnowledgePages } from "./knowledge.js";
import { coalesce, NO_READS, readsChanged } from "./viz/rerun.js";
import type { Reads } from "./tauri-bridge/generated/bindings.js";

/**
 * Shared in-memory page URI → title map. The markdown renderer uses
 * this to display `[[some-slug]]` wikilinks with the page's real title
 * instead of the raw id — readers should see "Local Snapshots" not
 * `local-snapshots`.
 *
 * One load on first subscribe; refreshed when a dataset the read read
 * changes (`readsChanged` over the read's own `reads` — the one rerun
 * rule, outside a component), once a burst of such commits goes quiet
 * (`coalesce`) — a scan storing its findings is many commits, and each
 * read lists every page. A change during a read is read once that one
 * ends. Components subscribe via `useKnowledgeTitle`.
 */

let titles = new Map<string, string>();
let reads: Reads = NO_READS;
let loaded = false;
let inFlight: Promise<void> | null = null;
/** A change arrived while a read was running: read again after it. */
let stale = false;
const listeners = new Set<() => void>();

function notify() {
  for (const fn of listeners) {
    try { fn(); } catch { /* ignore listener errors */ }
  }
}

async function refresh(): Promise<void> {
  if (inFlight) {
    stale = true;
    return inFlight;
  }
  stale = false;
  inFlight = (async () => {
    try {
      const result = await readKnowledgePages();
      const next = new Map<string, string>();
      for (const p of result.pages) {
        if (p.uri && p.title) next.set(p.uri, p.title);
      }
      titles = next;
      reads = result.reads;
      loaded = true;
      notify();
    } catch {
      // Leave the previous map in place on failure.
    } finally {
      inFlight = null;
    }
    if (stale) void refresh();
  })();
  return inFlight;
}

const refreshSoon = coalesce(() => void refresh());

let unsubscribeEvents: (() => void) | null = null;
function ensureSubscribed() {
  if (unsubscribeEvents) return;
  unsubscribeEvents = subscribeOxplowEvents((event) => {
    if (readsChanged(event as Record<string, unknown>, reads)) refreshSoon.schedule();
  });
}

/** Existence state of a ref target. `loading` = the page list hasn't
 *  loaded yet; `found` / `missing` once it has. The renderer shows
 *  `missing` as a broken, non-clickable link. */
export type RefStatus = "loading" | "found" | "missing";

/** Snapshot `{title, status}` for the page `uri`. Because the cache lists every
 *  page up front, once `loaded` a page absent from the map is a real
 *  `missing`; before that it's still `loading`. */
function knowledgeRefSnapshot(uri: string | null | undefined): { title: string | null; status: RefStatus } {
  if (!uri) return { title: null, status: "loading" };
  if (titles.has(uri)) return { title: titles.get(uri) ?? null, status: "found" };
  return { title: null, status: loaded ? "missing" : "loading" };
}

/**
 * Resolve a knowledge page's URI to its title AND existence status. `status`
 * is `loading` until the page list loads, then `found` / `missing`
 * (deleted page, stale wikilink). Backs the broken-link rendering.
 */
export function useKnowledgeRef(uri: string | null | undefined): { title: string | null; status: RefStatus } {
  const [state, setState] = useState(() => knowledgeRefSnapshot(uri));

  useEffect(() => {
    if (!uri) {
      setState({ title: null, status: "loading" });
      return;
    }
    ensureSubscribed();
    // Only a real change re-renders: a re-read that left this page's title
    // and status alone keeps the same state.
    const update = () =>
      setState((prev) => {
        const next = knowledgeRefSnapshot(uri);
        return prev.title === next.title && prev.status === next.status ? prev : next;
      });
    listeners.add(update);
    if (!loaded && !inFlight) {
      void refresh();
    } else {
      update();
    }
    return () => {
      listeners.delete(update);
    };
  }, [uri]);

  return state;
}

/** The same as `useKnowledgeRef`, awaited (a component frame's markdown, drawn
 *  once its titles are known). */
export async function loadKnowledgeRef(uri: string): Promise<{ title: string | null; status: RefStatus }> {
  ensureSubscribed();
  if (!loaded) await (inFlight ?? refresh());
  return knowledgeRefSnapshot(uri);
}

/**
 * Resolve a knowledge page's URI to its title. Returns `null` while the
 * cache is still loading or if the page isn't known (deleted page,
 * stale wikilink, etc.) — callers should fall back to the id in
 * that case.
 */
export function useKnowledgeTitle(uri: string | null | undefined): string | null {
  return useKnowledgeRef(uri).title;
}

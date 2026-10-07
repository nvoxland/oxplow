import { useEffect, useState } from "react";
import { subscribeOxplowEvents } from "./api.js";
import { readWikiPages } from "./knowledge.js";
import { coalesce, NO_READS, readsChanged } from "./lens/lensRerun.js";
import type { Reads } from "./tauri-bridge/generated/bindings.js";

/**
 * Shared in-memory slug → title map. The wiki body markdown renderer
 * uses this to display `[[some-slug]]` wikilinks with the page's real
 * title instead of the raw slug — readers should see "Local Snapshots"
 * not `local-snapshots`.
 *
 * One load on first subscribe; refreshed when a model the read read
 * changes (`readsChanged` over the read's own `reads` — the one rerun
 * rule, outside a component), once a burst of such commits goes quiet
 * (`coalesce`) — a scan storing its findings is many commits, and each
 * read lists every page. A change during a read is read once that one
 * ends. Components subscribe via `useWikiTitle`.
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
      const result = await readWikiPages();
      const next = new Map<string, string>();
      for (const p of result.pages) {
        if (p.slug && p.title) next.set(p.slug, p.title);
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

/** Snapshot `{title, status}` for `slug`. Because the cache lists every
 *  page up front, once `loaded` a slug absent from the map is a real
 *  `missing`; before that it's still `loading`. */
function wikiRefSnapshot(slug: string | null | undefined): { title: string | null; status: RefStatus } {
  if (!slug) return { title: null, status: "loading" };
  if (titles.has(slug)) return { title: titles.get(slug) ?? null, status: "found" };
  return { title: null, status: loaded ? "missing" : "loading" };
}

/**
 * Resolve a wiki slug to its page title AND existence status. `status`
 * is `loading` until the page list loads, then `found` / `missing`
 * (deleted page, stale wikilink). Backs the broken-link rendering.
 */
export function useWikiRef(slug: string | null | undefined): { title: string | null; status: RefStatus } {
  const [state, setState] = useState(() => wikiRefSnapshot(slug));

  useEffect(() => {
    if (!slug) {
      setState({ title: null, status: "loading" });
      return;
    }
    ensureSubscribed();
    // Only a real change re-renders: a re-read that left this slug's title
    // and status alone keeps the same state.
    const update = () =>
      setState((prev) => {
        const next = wikiRefSnapshot(slug);
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
  }, [slug]);

  return state;
}

/**
 * Resolve a wiki slug to its page title. Returns `null` while the
 * cache is still loading or if the slug isn't known (deleted page,
 * stale wikilink, etc.) — callers should fall back to the slug in
 * that case.
 */
export function useWikiTitle(slug: string | null | undefined): string | null {
  return useWikiRef(slug).title;
}

/**
 * Which pages can draw a URI, as the daemon says
 * (`.context/pages-and-tabs.md` "Which page draws a URI"): the window never
 * decides. The answer is asked once per identity (the URI without its `#`
 * view) and stream and kept as a promise, so a tab that is drawn again, or
 * a link opened a moment earlier (`prefetch`), asks nothing more.
 *
 * Every list is asked again whenever the extensions reload (a deploy, a
 * reconnect: the loaded `Extension[]` is a new array) because both the
 * pages and what they accept can have changed. The old list stands until the
 * new one arrives: "still asking" draws a tab's loading state, and that would
 * unmount an open tab with its terminal and any open menu. A list is not asked
 * again when a file's content changes, so a page whose `accepts.content` reads
 * it is judged on the content as of the first ask until the extensions reload.
 *
 * Routing stays synchronous: `peek` answers from what has settled, and a
 * tab whose list is still being fetched draws a brief loading state
 * (`undefined` here) until `useRenderers` hears it settle.
 */
import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";

import { unwrapResult } from "../api.js";
import { recordOpError } from "../opErrors.js";
import { commands, type Extension, type Renderer } from "../tauri-bridge/generated/bindings.js";
import { identityOf, typeOf } from "../uri/uri.js";

/** Whether a tab's URI is drawn by whichever page the daemon offers: all
 *  but a route (`oxplow/page`, which names its own page) and a symbol
 *  (which opens its file). */
export function drawnByRenderer(uri: string): boolean {
  const resource = typeOf(uri);
  return resource !== "oxplow/page" && resource !== "oxplow/symbol";
}

/** Asks the daemon (`renderers_for`). */
export type RenderersRpc = (uri: string, streamId: string | null) => Promise<Renderer[]>;

export const daemonRenderers: RenderersRpc = async (uri, streamId) => unwrapResult(await commands.renderersFor(uri, streamId));

export interface RenderersCache {
  /** What the daemon said for `uri`, once it has; `undefined` until then. */
  peek(extensions: readonly Extension[], uri: string, streamId: string | null): Renderer[] | undefined;
  /** Ask (or reuse the ask in flight) for `uri`. A failed ask is `[]`: the
   *  fallback page draws it, and the failure is recorded. */
  prefetch(extensions: readonly Extension[], uri: string, streamId: string | null): Promise<Renderer[]>;
  subscribe(listener: () => void): () => void;
  /** Changes whenever an ask settles. */
  version(): number;
}

export function createRenderersCache(rpc: RenderersRpc): RenderersCache {
  let epoch: readonly Extension[] | null = null;
  let version = 0;
  let asks = new Map<string, Promise<Renderer[]>>();
  const settled = new Map<string, Renderer[]>();
  const listeners = new Set<() => void>();
  const notify = () => {
    version++;
    for (const l of listeners) l();
  };
  /** The extensions reloaded: ask again. What has settled stands until then. */
  const within = (extensions: readonly Extension[]) => {
    if (epoch === extensions) return;
    epoch = extensions;
    asks = new Map();
  };
  const keyOf = (uri: string, streamId: string | null) => `${streamId ?? ""}\n${identityOf(uri) ?? uri}`;
  return {
    peek(extensions, uri, streamId) {
      within(extensions);
      return settled.get(keyOf(uri, streamId));
    },
    prefetch(extensions, uri, streamId) {
      within(extensions);
      const key = keyOf(uri, streamId);
      const have = asks.get(key);
      if (have) return have;
      const mine = epoch;
      const ask = rpc(identityOf(uri) ?? uri, streamId)
        .catch((e: unknown) => {
          recordOpError({ label: "Find renderers", message: e instanceof Error ? e.message : String(e) });
          return [] as Renderer[];
        })
        .then((list) => {
          // Answered after the extensions reloaded: not this epoch's.
          if (mine === epoch) {
            settled.set(key, list);
            notify();
          }
          return list;
        });
      asks.set(key, ask);
      return ask;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    version: () => version,
  };
}

/** The window's cache, over the daemon. */
export const renderersCache = createRenderersCache(daemonRenderers);

/**
 * The lists for `uris` (a URI without a `#` view, or with one: its
 * identity is the key), asked as the extensions and the uris change. Returns
 * a lookup that changes whenever one settles, so what depends on it
 * recomputes. `undefined` from the lookup: still being asked (or the
 * extensions are not loaded yet).
 */
export function useRenderers(
  extensions: readonly Extension[] | null,
  uris: readonly string[],
  streamId: string | null,
  cache: RenderersCache = renderersCache,
): (uri: string) => Renderer[] | undefined {
  const version = useSyncExternalStore(cache.subscribe, cache.version);
  const wanted = uris.join("\n");
  useEffect(() => {
    if (!extensions) return;
    for (const uri of wanted ? wanted.split("\n") : []) void cache.prefetch(extensions, uri, streamId);
  }, [cache, extensions, wanted, streamId]);
  const peek = useCallback(
    (uri: string) => (extensions ? cache.peek(extensions, uri, streamId) : undefined),
    [cache, extensions, streamId],
  );
  // A new function when something settled, so a memo over it recomputes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => (uri: string) => peek(uri), [peek, version]);
}

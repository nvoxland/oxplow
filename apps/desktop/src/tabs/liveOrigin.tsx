import { useEffect, useRef } from "react";

import { recordOpError } from "../opErrors.js";
import { resolveLiveOrigin } from "../viz/linkOrigin.js";
import { nodeById, nodeRunOf } from "../viz/nodeRun.js";
import { usePageRunsStore, usePageRunWith } from "../pageRuns.js";
import type { VizRun, PageRun } from "../tauri-bridge/generated/bindings.js";
import { identityOf, type LiveOrigin, type Origin, type OriginItem } from "./session.js";
import type { TabRef } from "./tabState.js";

/**
 * The origin a page's Up and Down step through. A stored origin is its own;
 * a live one (a page node's link column) takes its items from `items`,
 * the node's rows now, and its place from where `current` is in them. None
 * until the page has run, and none when the page is no longer in it.
 */
export function resolvedOrigin(origin: Origin, items: OriginItem[] | undefined, current: TabRef): Origin | null {
  if (!origin.live) return origin;
  if (!items) return null;
  const page = identityOf(current.id);
  const index = items.findIndex((i) => identityOf(i.ref.id) === page);
  return index === -1 ? null : { ...origin, items, index };
}

/** The items resolved for `slot`'s live `origin`, if they are of that origin. */
export function liveItemsOf(
  resolved: Record<string, { key: string; items: OriginItem[] }>,
  slot: string,
  origin: Origin | undefined,
): OriginItem[] | undefined {
  const got = resolved[slot];
  return origin?.live && got?.key === JSON.stringify(origin.live) ? got.items : undefined;
}

/** A live origin's list (a page node's link column, `<page>#<node>`) as
 *  its node's rows in the producing page's run; null when it names no
 *  node of a page, or the page no longer has the node. */
export function liveOriginNodeRun(live: LiveOrigin, run: PageRun): VizRun | null {
  const hash = live.node.indexOf("#");
  if (hash < 0 || run.page.id !== live.node.slice(0, hash)) return null;
  const node = nodeById(run.page, live.node.slice(hash + 1));
  return node ? nodeRunOf(run, node) : null;
}

/** Holds the producing page's run of a live origin, kept fresh by the
 *  daemon, handing the host its items each time it changes. Draws
 *  nothing. */
export function LiveOriginResolver({ live, streamId, onItems }: { live: LiveOrigin; streamId: string | null; onItems(items: OriginItem[]): void }) {
  const hash = live.node.indexOf("#");
  const store = usePageRunsStore();
  const state = usePageRunWith(store, hash < 0 ? null : { page: live.node.slice(0, hash), params: live.params, streamId, threadId: null });
  const onItemsRef = useRef(onItems);
  onItemsRef.current = onItems;
  const run = state.run;
  useEffect(() => {
    if (hash < 0) {
      onItemsRef.current([]);
      return;
    }
    if (!run) return;
    const nodeRun = liveOriginNodeRun(live, run);
    onItemsRef.current(nodeRun ? resolveLiveOrigin(nodeRun, live.column) : []);
    // `live` is read through the run it keyed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run, hash]);
  useEffect(() => {
    if (state.error) recordOpError({ label: "Up and Down", message: state.error });
  }, [state.error]);
  return null;
}

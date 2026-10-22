/**
 * Where a URI opens (`.context/pages-and-tabs.md` "Linking between tabs"):
 * the one place that decides it. A URI opens as itself — its tab's id is
 * the URI, drawn by the page that renders it — except a stream's or a
 * thread's, which selects it. It says what the window does with the
 * result: open a tab, select a thread or a stream, or nothing.
 *
 * What it decides is *what* opens; how (in this tab, a new one) stays the
 * click's (`RouteLink`, `useRouteDispatch`).
 */
import { idOf } from "../uri/uri.js";
import { refFromTabId } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

export type RefTarget =
  | { tab: TabRef }
  | { thread: string }
  | { stream: string }
  | { unopenable: string };

/** What the window does with tab id `tabId`. */
function targetOf(tabId: string): RefTarget {
  const thread = idOf(tabId, "oxplow/thread");
  if (thread !== null) return { thread };
  const stream = idOf(tabId, "oxplow/stream");
  if (stream !== null) return { stream };
  const tab = refFromTabId(tabId);
  return tab ? { tab } : { unopenable: `\`${tabId}\` isn't a URI the window opens` };
}

/** Where `ref` opens: as itself — a tab whose page is the one that renders
 *  it (`pages/renderers.ts`; one nothing renders opens on the fallback) —
 *  or, for a stream's or a thread's URI, selecting it. Only text that
 *  isn't a URI opens nothing. */
export function refTargetNow(ref: string): RefTarget {
  return targetOf(ref);
}

/** Where `ref` opens. */
export async function refTarget(ref: string): Promise<RefTarget> {
  return refTargetNow(ref);
}


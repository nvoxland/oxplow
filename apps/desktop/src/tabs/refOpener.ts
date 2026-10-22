/**
 * Where a URI opens (`.context/pages-and-tabs.md` "Linking between tabs"):
 * the one place that decides it. An extension's resource names the page that
 * shows one (`v_resource.page`), which opens with the URI as its `uri`
 * param; every other URI opens through `refFromTabId`. It says what the
 * window does with the result: open a tab, select a thread or a stream, or
 * nothing.
 *
 * What it decides is *what* opens; how (in this tab, a new one) stays the
 * click's (`RouteLink`, `useRouteDispatch`).
 */
import { resourceInfo, resourcePageTabId } from "../resources.js";
import { idOf, typeOf } from "../uri/uri.js";
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
  return tab ? { tab } : { unopenable: `\`${tabId}\` isn't a page the window shows` };
}

/** Where `ref` opens. Before the registry is read (a window starting) a
 *  URI opens as itself, which is where every core resource but a few
 *  opens. */
export function refTargetNow(ref: string): RefTarget {
  const resource = typeOf(ref);
  if (resource === null) return { unopenable: noPage(ref) };
  const info = resourceInfo(resource);
  if (!info) return targetOf(ref);
  const tabId = resourcePageTabId(ref);
  if (tabId !== null) return targetOf(tabId);
  // One of oxplow's own (`oxplow/…`, whoever declares it) opens as itself;
  // an extension's with no page has nowhere to open.
  return resource.startsWith("oxplow/") ? targetOf(ref) : { unopenable: noPage(ref) };
}

/** Where `ref` opens. */
export async function refTarget(ref: string): Promise<RefTarget> {
  return refTargetNow(ref);
}

function noPage(ref: string): string {
  return `\`${ref}\` has no page`;
}

/**
 * Opening a ref at a click: the one path every link to a ref of any kind
 * takes (`.context/pages-and-tabs.md` "Linking between tabs"). `refOpener`
 * says where it opens (its kind's page template in the registry); this
 * does it through the window's `RefHost`: a tab in the current page or a
 * new one, a selected thread, a shown stream, or an error naming why not.
 * How (in this tab, a new one) is the click's (`RouteLink`).
 */
import { refTarget, type RefTarget } from "./refOpener.js";
import type { NavSiblings } from "./PageNavigationContext.js";
import type { TabRef } from "./tabState.js";

/** How a click asks a page to open. */
export interface OpenHow {
  /** A new tab instead of this one. */
  newTab?: boolean;
  /** The list the link sits in, for prev/next on the page it opens. */
  siblings?: NavSiblings;
  /** In place of the current page, which leaves history: Back won't
   *  return to it (a submitted form gives way to what it made). */
  replace?: boolean;
}

/** What the window does with where a ref opens. */
export interface RefHost {
  /** Show `tab`, in the current tab or a new one. */
  open(tab: TabRef, how: OpenHow): void;
  /** Select thread `thread`; false when no stream has it. */
  showThread(thread: string): boolean | Promise<boolean>;
  /** Show stream `stream`; false when there is none. */
  showStream(stream: string): boolean | Promise<boolean>;
  /** A ref that didn't open, and why. */
  failed(message: string): void;
}

/** Open `ref` through `host`: where its kind's page says. */
export async function openRef(
  ref: string,
  how: OpenHow,
  host: RefHost,
  resolve: (ref: string) => Promise<RefTarget> = refTarget,
): Promise<void> {
  let target: RefTarget;
  try {
    target = await resolve(ref);
  } catch (e) {
    host.failed(e instanceof Error ? e.message : String(e));
    return;
  }
  if ("tab" in target) {
    host.open(target.tab, how);
  } else if ("thread" in target) {
    if (!(await host.showThread(target.thread))) host.failed(`no thread \`${target.thread}\``);
  } else if ("stream" in target) {
    if (!(await host.showStream(target.stream))) host.failed(`no stream \`${target.stream}\``);
  } else {
    host.failed(target.unopenable);
  }
}

/** Opens a ref from outside any page (the rail, the palette): the
 *  window's, set by `App`. */
let windowOpener: ((ref: string, how: OpenHow) => void) | null = null;

export function setWindowRefOpener(open: ((ref: string, how: OpenHow) => void) | null): void {
  windowOpener = open;
}

export function openRefInWindow(ref: string, how: OpenHow): void {
  windowOpener?.(ref, how);
}

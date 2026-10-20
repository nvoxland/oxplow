import { createContext, useContext, useEffect } from "react";
import type { TabRef } from "./tabState.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { BookmarkScope } from "./bookmarks.js";
import type { Origin } from "./session.js";

export interface NavigateOptions {
  /** Force a brand new tab (slot) instead of replacing the current page. */
  newTab?: boolean;
  /**
   * The collection this navigation came from. The destination keeps it as
   * its entry's origin: its nav bar steps through the items (Up and Down)
   * without touching Back and Forward. The destination's position is
   * found by the item whose `ref.id` is the destination's; `index` is the
   * fallback.
   */
  origin?: Origin;
  /** In place of the current page, which leaves history: Back won't
   *  return to it (a submitted form gives way to what it made). */
  replace?: boolean;
}

export interface BookmarkBinding {
  /** The scope this page is bookmarked at, as this thread sees it. */
  scope: BookmarkScope | null;
  /** Bookmark it at `scope` (moving it there), or take it off when it's
   *  already there. */
  toggle(scope: BookmarkScope): void;
}

export interface PageNavigation {
  /**
   * Navigate to `ref`. Default is in-tab navigation: replaces the
   * current page in the active tab and pushes the old page onto the
   * back stack. When `newTab` is true, opens in a new tab.
   *
   * Outside of a page body (rail HUD, command palette), the host
   * implementation defaults to `newTab: true` semantics regardless of
   * the option, since there's no "current" tab to navigate within.
   */
  navigate(ref: TabRef, opts?: NavigateOptions): void;
  /**
   * Open the ref `ref` (of any kind) where its kind's page says: a tab
   * here or a new one, a selected thread, a shown stream
   * (`tabs/openRef.ts`). The one way a link to a ref string opens.
   */
  openRef?(ref: string, how?: NavigateOptions): void;
  goBack(): void;
  goForward(): void;
  canGoBack: boolean;
  canGoForward: boolean;
  /** The tab is pinned (its kind's, `isPinnedKind`): it leads the strip
   *  and is always itself, so its bar has no back or forward. */
  pinned?: boolean;
  /** The collection the current page was opened from; null when it was
   *  not opened from one. Up and Down step through its items without
   *  affecting back/forward history. */
  origin?: Origin | null;
  /** Show item `index` of the origin in this page's place. Out-of-range
   *  or the current item is a no-op. */
  stepOrigin?(index: number): void;
  /** Bookmark binding for the page currently rendered in this tab. */
  bookmark?: BookmarkBinding;
  /** The page's canonical ref (`commit:<sha>`, `lens:x/y`) and stream,
   *  for the Ask menu; absent for a page whose tab id isn't one. */
  ask?: import("../components/Prompts/AskMenu.js").AskTarget;
  /**
   * Register the page's current title with the host so the tab strip
   * label and the shared chrome header pull from a single source.
   * Pages call this through the `usePageTitle(...)` helper rather than
   * directly. Optional so existing pages that pass `title` to `Page`
   * keep working unchanged.
   */
  setTitle?(title: string): void;
  /**
   * The current title registered for this tab — populated by the host
   * after a `setTitle` call. `Page` reads this when no explicit
   * `title` prop is supplied.
   */
  title?: string;
  /** The page this body draws: its entry's ref (`tabs/session.ts`). */
  ref?: TabRef;
  /** **View params** — this tab's address: what the page shows (its
   *  filters, its search, the row picked). Shareable: a bookmark, a
   *  reopened tab and an agent see them. */
  viewParams?: Record<string, SqlCell>;
  /**
   * Show `params` as the page's view params (the whole set; the values at
   * their defaults left out). A filter replaces the tab's entry; a
   * selection (`push`) adds one, so Back undoes it. Absent on a body that
   * isn't the tab's current page.
   */
  setViewParams?(params: Record<string, SqlCell>, opts?: { push?: boolean }): void;
  /** **History state** — this step of this tab's history (`Entry.historyState`):
   *  scroll, an open section, a draft; back with Back and after a
   *  restart, never shared. Read and written through `useHistoryState`. */
  historyState?: Record<string, unknown>;
  /** Merge `patch` into the history state. Absent on a body that isn't
   *  the tab's current page. */
  setHistoryState?(patch: Record<string, unknown>): void;
}

export const PageNavigationContext = createContext<PageNavigation | null>(null);

/** Read the active page's navigation API. Throws if used outside a Provider. */
export function usePageNavigation(): PageNavigation {
  const ctx = useContext(PageNavigationContext);
  if (!ctx) throw new Error("usePageNavigation called outside PageNavigationContext");
  return ctx;
}

/** Optional read — returns null when there's no provider (e.g., rail HUD). */
export function useOptionalPageNavigation(): PageNavigation | null {
  return useContext(PageNavigationContext);
}

/**
 * Register the current page's title with the host tab. Called by every
 * page that wants its title to surface in the shared chrome and in the
 * tab strip without owning duplicate header markup. Safe to call from
 * components rendered outside a provider — it just no-ops.
 */
export function usePageTitle(title: string | null | undefined): void {
  const ctx = useContext(PageNavigationContext);
  const set = ctx?.setTitle;
  useEffect(() => {
    if (!set) return;
    if (title == null || title === "") return;
    set(title);
  }, [set, title]);
}

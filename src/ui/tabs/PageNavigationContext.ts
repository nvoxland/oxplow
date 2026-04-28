import { createContext, useContext, useEffect } from "react";
import type { TabRef } from "./tabState.js";
import type { BookmarkScope } from "./bookmarks.js";

export interface NavigateOptions {
  /** Force a brand new tab (slot) instead of replacing the current page. */
  newTab?: boolean;
}

export interface BookmarkBinding {
  /** All scopes this page is currently bookmarked at. */
  scopes: BookmarkScope[];
  /** Toggle bookmark in the given scope. */
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
  goBack(): void;
  goForward(): void;
  canGoBack: boolean;
  canGoForward: boolean;
  /** Bookmark binding for the page currently rendered in this tab. */
  bookmark?: BookmarkBinding;
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

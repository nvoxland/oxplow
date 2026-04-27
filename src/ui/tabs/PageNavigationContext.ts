import { createContext, useContext } from "react";
import type { TabRef } from "./tabState.js";
import type { BookmarkScope } from "./bookmarks.js";

export interface NavigateOptions {
  /** Force a brand new tab (slot) instead of replacing the current page. */
  newTab?: boolean;
}

export interface BookmarkBinding {
  /** All scopes this page is currently bookmarked at. */
  scopes: BookmarkScope[];
  /** Toggle bookmark in the given scope (or the user's last-used
   *  scope if omitted). */
  toggle(scope?: BookmarkScope): void;
  /** Persisted "last-used" scope, drives the default-click behavior. */
  lastScope: BookmarkScope;
  setLastScope(scope: BookmarkScope): void;
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

import { createContext, useContext, useEffect, useRef } from "react";
import type { TabRef } from "./tabState.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { BookmarkScope } from "./bookmarks.js";
import type { Origin } from "./session.js";
import type { PageAction } from "../kit/actions.js";

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
  /** The new entry's history state: what it shows that is no address
   *  (a compare's two texts). With `newTab`. */
  state?: Record<string, unknown>;
}

export interface BookmarkBinding {
  /** The scope this page is bookmarked at, as this thread sees it. */
  scope: BookmarkScope | null;
  /** Bookmark it at `scope` (moving it there), or take it off when it's
   *  already there. */
  toggle(scope: BookmarkScope): void;
  /** The same page is bookmarked at other values (another view); the scope
   *  of that bookmark, and what moves it to this view. */
  other?: { scope: BookmarkScope } | null;
  updateOther?(): void;
}

/** What a page body that edits tells its tab (`useTabEditing`). */
export interface TabEditing {
  /** It holds edits not yet saved: the tab says ●, the menu's Save is
   *  offered (`pageDirty`), and the tab cap never closes it. */
  dirty: boolean;
  /** Save them (the menu's Save, Cmd/Ctrl+S outside the body). */
  save(): void;
  /** Find in what it edits (the menu's Find), when it can. */
  find?(): void;
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
  /** Close this page's tab (a page for what was just deleted). */
  close?(): void;
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
  /** The page's canonical ref (`commit:<sha>`, `page:x/y`) and stream,
   *  for the Ask menu; absent for a page whose tab id isn't one. */
  ask?: import("../components/Prompts/askTarget.js").AskTarget;
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
  /** Tell the tab whether its page holds unsaved edits and how to save
   *  them; `null` when it no longer edits. Through `useTabEditing`.
   *  Absent on a body that isn't the tab's current page. */
  setEditing?(editing: TabEditing | null): void;
  /** Put the page's actions on its tab's right-click menu too (the same
   *  ones its nav bar draws); `null` when it has none. Through
   *  `useTabActions`. Absent on a body that isn't the tab's current page. */
  setTabActions?(actions: readonly PageAction[] | null): void;
  /** The page this body draws: its entry's ref (`tabs/session.ts`). */
  ref?: TabRef;
  /** The pages that can draw this body's ref, as the daemon says
   *  (`pages/useRenderers.ts`): what Open With lists. Absent for a route. */
  renderers?: readonly import("../tauri-bridge/generated/bindings.js").Renderer[];
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
  /** Merge `patch` into the page's view params (`undefined` drops a key),
   *  against the tab's current entry, so two changes in one tick both
   *  land. What `useViewParam` and its siblings write through. */
  patchViewParams?(patch: Record<string, SqlCell | undefined>, opts?: { push?: boolean }): void;
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

/**
 * Tell the page's tab what it is editing: `dirty` while it holds edits
 * not yet saved, and `save` to write them. The shell draws the tab's ●,
 * offers Save (`pageDirty`), runs it from the menu, and keeps the tab
 * from the tab cap. The latest `save` is the one run. A no-op outside a
 * tab.
 */
export function useTabEditing({ dirty, save, find }: TabEditing): void {
  const set = useContext(PageNavigationContext)?.setEditing;
  const saveRef = useRef(save);
  saveRef.current = save;
  const findRef = useRef(find);
  findRef.current = find;
  const finds = !!find;
  useEffect(() => {
    if (!set) return;
    set({ dirty, save: () => saveRef.current(), ...(finds ? { find: () => findRef.current?.() } : {}) });
  }, [set, dirty, finds]);
  useEffect(() => {
    if (!set) return;
    return () => set(null);
  }, [set]);
}

/**
 * Put the page's actions on its tab's right-click menu: one source for the
 * nav bar and the tab (an agent session's Rename…, Fork Conversation…). The latest
 * actions are the ones run. A no-op outside a tab.
 */
export function useTabActions(actions: readonly PageAction[]): void {
  const set = useContext(PageNavigationContext)?.setTabActions;
  const key = actions.map((a) => `${a.id}:${a.label}:${a.disabled ? 1 : 0}`).join("|");
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    if (!set) return;
    set(latest.current.length > 0 ? latest.current.map((a) => ({ ...a, run: () => latest.current.find((b) => b.id === a.id)?.run() })) : null);
  }, [set, key]);
  useEffect(() => {
    if (!set) return;
    return () => set(null);
  }, [set]);
}

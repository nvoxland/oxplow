import type { CSSProperties, MouseEvent, ReactNode } from "react";
import { useCallback, useMemo } from "react";
import type { TabRef } from "./tabState.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";

/**
 * Browser-style click semantics for any element that points at a
 * `TabRef`. Returns a `dispatch(newTab)` function plus a bundle of
 * mouse event handlers that map plain-click → in-tab navigate,
 * cmd/ctrl-click + middle-click + right-click → open in a new tab.
 *
 * Use this from `RouteLink` (button) and from list/tree rows that
 * need to keep their existing `<div>` markup (file tree rows, note
 * rows, …) — wiring through the same dispatch keeps the contract
 * consistent without forcing every row into a button.
 *
 * Outside a `PageNavigationContext` provider, `onNavigate` MUST be
 * supplied — there's no current tab to navigate within.
 */
export function useRouteDispatch(
  ref: TabRef,
  options: {
    onNavigate?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
    pinnedSlot?: boolean;
  } = {},
) {
  const { onNavigate, pinnedSlot = false } = options;
  const ctxNav = useOptionalPageNavigation();
  const dispatch = useCallback((newTab: boolean) => {
    const escape = newTab || pinnedSlot;
    // Context-first: when this link sits inside a page that supplies a
    // `PageNavigationContext`, route through it so plain-click does in-
    // tab navigation. Outside a page (rail / palette), fall back to the
    // caller's `onNavigate` — that callback typically maps to the
    // host's "always-open-as-new-tab" handler.
    if (ctxNav) {
      ctxNav.navigate(ref, { newTab: escape });
      return;
    }
    if (onNavigate) {
      onNavigate(ref, { newTab: escape });
      return;
    }
    // No way to navigate — silently no-op.
  }, [ctxNav, onNavigate, pinnedSlot, ref]);

  const handlers = useMemo(() => ({
    onClick: (e: MouseEvent) => {
      const newTab = e.metaKey || e.ctrlKey;
      dispatch(newTab);
    },
    onAuxClick: (e: MouseEvent) => {
      // Middle-click → new tab. Browser default behavior.
      if (e.button === 1) {
        e.preventDefault();
        dispatch(true);
      }
    },
    onContextMenu: (e: MouseEvent) => {
      // Right-click → open in new tab. A richer popover (Open / Open
      // in new tab / Copy link) is planned but the consistent
      // "right-click never destroys the current view" contract is
      // honored by routing this to the new-tab path for now.
      e.preventDefault();
      dispatch(true);
    },
  }), [dispatch]);

  return { dispatch, handlers };
}

export interface RouteLinkProps {
  ref: TabRef;
  children: ReactNode;
  /**
   * Fallback navigation — used only when no `PageNavigationContext`
   * is present (rail HUD, palette). Inside a page, the link always
   * routes through the context so plain-click does in-tab nav and
   * modifier/right-click escape to a new tab. Pass this so the same
   * link works outside a page too.
   */
  onNavigate?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
  /**
   * When true, this link belongs to a "pinned" page kind (Files,
   * Notes, etc.) — clicks always open in its canonical slot rather
   * than navigating in-tab. The host's `onNavigate` is responsible
   * for honoring this hint; `RouteLink` simply forwards it via
   * `newTab: true` for the default-context flow.
   */
  pinnedSlot?: boolean;
  className?: string;
  style?: CSSProperties;
  testId?: string;
  title?: string;
  draggable?: boolean;
  onDragStart?: (e: React.DragEvent) => void;
}

/**
 * Browser-style link to another page. Click semantics:
 *  - Left-click → in-tab navigation (or new tab when `pinnedSlot`)
 *  - Cmd/Ctrl-click, middle-click → new tab
 *  - Right-click → today: also new tab (popover menu lives in
 *    Phase 1.5; this stub keeps the contract consistent)
 *
 * Outside a `PageNavigationContext` provider, `onNavigate` MUST be
 * supplied — there's no current tab to navigate within.
 */
export function RouteLink({
  ref,
  children,
  onNavigate,
  pinnedSlot = false,
  className,
  style,
  testId,
  title,
  draggable,
  onDragStart,
}: RouteLinkProps) {
  const { handlers } = useRouteDispatch(ref, { onNavigate, pinnedSlot });
  return (
    <button
      type="button"
      data-testid={testId}
      title={title}
      draggable={draggable}
      onDragStart={onDragStart}
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      className={className}
      style={style}
    >
      {children}
    </button>
  );
}

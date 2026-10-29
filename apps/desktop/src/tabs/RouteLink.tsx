import type { CSSProperties, MouseEvent, ReactNode } from "react";
import { useMemo } from "react";
import type { TabRef } from "./tabState.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";
import type { PageNavigation } from "./PageNavigationContext.js";
import type { Origin } from "./session.js";
import { openRefInWindow } from "./openRef.js";

/**
 * Browser-style click semantics for any element that points at a page:
 * a `TabRef`, or a ref string of any kind, which opens where its kind's
 * page says (`openRef`; a note opens its thread). Returns a `dispatch(newTab)` function plus a bundle of
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
export interface RouteDispatchOptions {
  onNavigate?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
  pinnedSlot?: boolean;
  /**
   * Optional collection this row is part of. When the row
   * dispatches an in-tab navigation, the origin travels with it
   * and the destination page picks up prev/next buttons in its
   * nav bar. Ignored when the navigation falls back to a new tab
   * or to the rail-side `onNavigate` callback.
   */
  origin?: Origin;
}

/**
 * The one dispatch (`dispatch(newTab)`) behind every link, with no hook:
 * a row built in a loop (a table's `rowProps`) makes its own from the
 * page's navigation context. Context first: inside a page, plain-click
 * navigates in-tab; outside one (rail / palette) the caller's `onNavigate`
 * runs, which typically opens a new tab.
 */
export function routeDispatch(
  ref: TabRef | string,
  ctxNav: PageNavigation | null,
  { onNavigate, pinnedSlot = false, origin }: RouteDispatchOptions = {},
): (newTab: boolean) => void {
  return (newTab) => {
    const escape = newTab || pinnedSlot;
    // A ref string opens through the registry, wherever the link sits.
    if (typeof ref === "string") {
      const how = { newTab: escape, origin: escape ? undefined : origin };
      if (ctxNav?.openRef) ctxNav.openRef(ref, how);
      else openRefInWindow(ref, how);
      return;
    }
    if (ctxNav) {
      ctxNav.navigate(ref, { newTab: escape, origin: escape ? undefined : origin });
      return;
    }
    if (onNavigate) {
      onNavigate(ref, { newTab: escape });
      return;
    }
    // No way to navigate — silently no-op.
  };
}

/** Browser-style click handlers for `dispatch`: plain → in this tab,
 *  Cmd/Ctrl-click, middle- and right-click → a new tab. */
export function routeHandlers(dispatch: (newTab: boolean) => void) {
  return {
    onClick: (e: MouseEvent) => dispatch(e.metaKey || e.ctrlKey),
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
  };
}

export function useRouteDispatch(ref: TabRef | string, options: RouteDispatchOptions = {}) {
  const { onNavigate, pinnedSlot = false, origin } = options;
  const ctxNav = useOptionalPageNavigation();
  const dispatch = useMemo(
    () => routeDispatch(ref, ctxNav, { onNavigate, pinnedSlot, origin }),
    [ref, ctxNav, onNavigate, pinnedSlot, origin],
  );
  const handlers = useMemo(() => routeHandlers(dispatch), [dispatch]);
  return { dispatch, handlers };
}

export interface RouteLinkProps {
  // NOT `ref` — that's a reserved prop React 18 strips from function components
  // (it never reaches this component), so the destination MUST use another name.
  to: TabRef | string;
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
  /** The collection this link is one of: the opened page's Up and Down step through it. */
  origin?: Origin;
  className?: string;
  style?: CSSProperties;
  testId?: string;
  title?: string;
  /** The accessible name, when the children (an icon) carry no text. */
  ariaLabel?: string;
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
  to,
  children,
  onNavigate,
  pinnedSlot = false,
  origin,
  className,
  style,
  testId,
  title,
  ariaLabel,
  draggable,
  onDragStart,
}: RouteLinkProps) {
  const { handlers } = useRouteDispatch(to, { onNavigate, pinnedSlot, origin });
  return (
    <button
      type="button"
      data-testid={testId}
      title={title}
      aria-label={ariaLabel}
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

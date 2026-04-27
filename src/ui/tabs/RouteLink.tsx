import type { CSSProperties, MouseEvent, ReactNode } from "react";
import type { TabRef } from "./tabState.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";

export interface RouteLinkProps {
  ref: TabRef;
  children: ReactNode;
  /**
   * Override where this link routes. Defaults to using the active
   * `PageNavigation` from context (in-tab navigation). Use this when
   * the link lives outside a page body (rail HUD, palette) and the
   * host wants to force "always open in a new tab" semantics.
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
  const ctxNav = useOptionalPageNavigation();
  const dispatch = (newTab: boolean) => {
    if (onNavigate) {
      onNavigate(ref, { newTab: newTab || pinnedSlot });
      return;
    }
    if (ctxNav) {
      ctxNav.navigate(ref, { newTab: newTab || pinnedSlot });
      return;
    }
    // No way to navigate — silently no-op rather than throw, since
    // RouteLinks are sprinkled across the UI and a missing provider
    // shouldn't crash a page.
  };

  return (
    <button
      type="button"
      data-testid={testId}
      title={title}
      draggable={draggable}
      onDragStart={onDragStart}
      onClick={(e: MouseEvent) => {
        const newTab = e.metaKey || e.ctrlKey;
        dispatch(newTab);
      }}
      onAuxClick={(e: MouseEvent) => {
        // Middle-click (button 1) → new tab. Browser default behavior.
        if (e.button === 1) {
          e.preventDefault();
          dispatch(true);
        }
      }}
      onContextMenu={(e: MouseEvent) => {
        // Right-click → open in new tab. A richer popover (Open / Open
        // in new tab / Copy link) is planned but the consistent
        // "right-click never destroys the current view" contract is
        // honored by routing this to the new-tab path for now.
        e.preventDefault();
        dispatch(true);
      }}
      className={className}
      style={style}
    >
      {children}
    </button>
  );
}

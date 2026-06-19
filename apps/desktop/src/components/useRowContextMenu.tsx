import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
import { useCallback, useState } from "react";

import type { MenuItem } from "../menu.js";
import { ContextMenu } from "./ContextMenu.js";

/**
 * Right-click affordance for per-row actions — the replacement for the
 * visible `⋯` kebab button the IA redesign once used. It opens the shared
 * {@link ContextMenu} popover with the same `MenuItem[]` payload the kebab
 * carried; we've committed back to right-click (see `.context/usability.md`)
 * so rows reclaim the kebab's horizontal space.
 *
 * Two shapes, one popover:
 *  - {@link useContextMenu} — call once in a list/parent that renders rows in
 *    a `.map()` (where a per-row hook can't run). Each row does
 *    `onContextMenu={(e) => open(e, items)}`; render `{menu}` once.
 *  - {@link useRowContextMenu} — bind the items up front when the row is its
 *    own component; spread `onContextMenu`/`onKeyDown` and render `{menu}`.
 *
 * The OS-native menu is already cancelled globally by
 * `installContextMenuSuppressor`; these handlers also cancel it locally so
 * the same code works in a plain browser (where suppression is just
 * `preventDefault` on `contextmenu` — standard DOM, no Tauri dependency).
 *
 * Keyboard parity: the key handler opens the same menu on the Menu key or
 * Shift+F10 for the focused row, anchored at its bounding rect, so
 * keyboard-first users never need the mouse.
 */
export function useContextMenu(): {
  open(event: MouseEvent, items: MenuItem[]): void;
  openForKey(event: KeyboardEvent, items: MenuItem[]): void;
  menu: ReactNode;
} {
  const [state, setState] = useState<{ pos: { x: number; y: number }; items: MenuItem[] } | null>(null);

  const open = useCallback((event: MouseEvent, items: MenuItem[]) => {
    // Always cancel the native menu on a surface that owns its own actions,
    // even when there are none to show, so the OS menu never leaks here.
    event.preventDefault();
    event.stopPropagation();
    if (items.length === 0) return;
    setState({ pos: { x: event.clientX, y: event.clientY }, items });
  }, []);

  const openForKey = useCallback((event: KeyboardEvent, items: MenuItem[]) => {
    const isMenuKey = event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
    if (!isMenuKey || items.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    // Anchor under the focused row, left-aligned to it.
    const rect = event.currentTarget.getBoundingClientRect();
    setState({ pos: { x: rect.left, y: rect.bottom + 2 }, items });
  }, []);

  const menu = state ? (
    <ContextMenu items={state.items} position={state.pos} onClose={() => setState(null)} />
  ) : null;

  return { open, openForKey, menu };
}

/**
 * Items-bound convenience wrapper around {@link useContextMenu} for rows that
 * are their own component.
 *
 * Usage:
 *   const { onContextMenu, onKeyDown, menu } = useRowContextMenu(items);
 *   return <div onContextMenu={onContextMenu} onKeyDown={onKeyDown}>… {menu}</div>;
 */
export function useRowContextMenu(items: MenuItem[]): {
  onContextMenu(event: MouseEvent): void;
  onKeyDown(event: KeyboardEvent): void;
  menu: ReactNode;
} {
  const { open, openForKey, menu } = useContextMenu();
  const onContextMenu = useCallback((event: MouseEvent) => open(event, items), [open, items]);
  const onKeyDown = useCallback((event: KeyboardEvent) => openForKey(event, items), [openForKey, items]);
  return { onContextMenu, onKeyDown, menu };
}

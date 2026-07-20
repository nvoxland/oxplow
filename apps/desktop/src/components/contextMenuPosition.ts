// Viewport-fitting math for `ContextMenu` (tsk146). Pure and React-free so the
// off-screen cases are unit-testable — the happy-dom harness does no layout, so
// a rendered menu reports a zero-sized rect and a "is the top on screen?"
// assertion would pass vacuously.

/** Gap kept between a menu and the viewport edge. */
export const MENU_MARGIN = 8;

/** The tallest a menu may be before it scrolls internally. */
export const MENU_MAX_HEIGHT = `calc(100vh - ${MENU_MARGIN * 2}px)`;

/**
 * Clamp a menu's top-left so the whole box stays on screen.
 *
 * When the menu is taller (or wider) than the viewport, the available range
 * collapses and the menu is pinned at the margin — its overflow is then handled
 * by scrolling, not by moving. That only holds because the menu's height is
 * bounded by {@link MENU_MAX_HEIGHT}; without it, a tall menu pins to the top
 * and the items past the fold are simply unreachable.
 */
export function clampMenuPosition(
  pos: { x: number; y: number },
  size: { width: number; height: number },
  viewport: { width: number; height: number },
  margin = MENU_MARGIN,
): { x: number; y: number } {
  const maxX = Math.max(margin, viewport.width - size.width - margin);
  const maxY = Math.max(margin, viewport.height - size.height - margin);
  return {
    x: Math.min(Math.max(margin, pos.x), maxX),
    y: Math.min(Math.max(margin, pos.y), maxY),
  };
}

/**
 * How far to shift a submenu vertically so it fits, relative to its anchor.
 *
 * A submenu opens level with its parent item, so a long one runs off the
 * bottom. The original fix shifted it up by exactly the overflow — which, for a
 * submenu taller than the viewport, pushed its TOP off the screen instead and
 * made the first entries unreachable. That's the reported bug: a ~30-item
 * submenu whose opening items were cut off above the window.
 *
 * The shift is now floored so the top never crosses the margin. Bounding the
 * height via {@link MENU_MAX_HEIGHT} makes that floor unreachable in practice,
 * but it is kept because the two must not silently disagree: if a caller ever
 * escapes the max-height, the menu still stays on screen and scrolls.
 */
export function submenuTopOffset(
  rect: { top: number; height: number },
  viewportHeight: number,
  margin = MENU_MARGIN,
): number {
  const overflowBottom = rect.top + rect.height - (viewportHeight - margin);
  if (overflowBottom <= 0) return 0;
  return Math.max(-overflowBottom, margin - rect.top);
}

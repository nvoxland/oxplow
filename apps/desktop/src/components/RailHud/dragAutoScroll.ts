/// Scrolling a scroll box while something is dragged near its top or bottom
/// edge, so a drop target out of view can be reached (the rail is often
/// taller than the window). Browsers don't do this for an overflow box on
/// their own, WebKit least of all.
import { useEffect, useRef, type RefObject } from "react";

/** How far in from an edge the scrolling starts, in px. */
export const EDGE = 48;
/** The fastest it scrolls, in px per frame. */
export const MAX_SPEED = 18;

/** Pixels to scroll per frame for a pointer at `y` in a box spanning
 *  `rect`: negative near the top, positive near the bottom, faster closer
 *  to the edge, the fastest past it; 0 elsewhere. */
export function edgeScrollSpeed(y: number, rect: { top: number; bottom: number }): number {
  const fromTop = y - rect.top;
  if (fromTop < EDGE) return -Math.round(MAX_SPEED * Math.min(1, (EDGE - Math.max(0, fromTop)) / EDGE));
  const fromBottom = rect.bottom - y;
  if (fromBottom < EDGE) return Math.round(MAX_SPEED * Math.min(1, (EDGE - Math.max(0, fromBottom)) / EDGE));
  return 0;
}

/** While `active` (a drag of the box's own items), scroll `box` each frame
 *  by the speed the drag's last position gives. The document's `dragover`
 *  says where it is, so a drag just above or below the box (over the
 *  search box, the window's edge) still scrolls it; one beside it doesn't. */
export function useDragAutoScroll(box: RefObject<HTMLElement | null>, active: boolean): void {
  const speed = useRef(0);
  useEffect(() => {
    if (!active) return;
    const onDrag = (e: DragEvent) => {
      const el = box.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      speed.current = e.clientX < rect.left || e.clientX > rect.right ? 0 : edgeScrollSpeed(e.clientY, rect);
    };
    document.addEventListener("dragover", onDrag);
    let frame = requestAnimationFrame(function step() {
      const el = box.current;
      if (el && speed.current !== 0) el.scrollTop += speed.current;
      frame = requestAnimationFrame(step);
    });
    return () => {
      document.removeEventListener("dragover", onDrag);
      cancelAnimationFrame(frame);
      speed.current = 0;
    };
  }, [box, active]);
}

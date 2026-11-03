/// Whether an element is on screen: a tab body that isn't shown is
/// `display: none`, so its element has no size. What a terminal opens on
/// (an agent's tab spawns nothing until it is shown) and what takes the
/// keys and the agent input bus.
import { useEffect, useState, type RefObject } from "react";

export function useShown(ref: RefObject<HTMLElement | null>): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setShown(el.clientWidth > 1 && el.clientHeight > 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return shown;
}

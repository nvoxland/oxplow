import { useEffect, useState } from "react";

/** Below this window width the rail collapses so the page keeps the room. */
export const NARROW_WINDOW_PX = 800;

export function isNarrowWindow(width: number): boolean {
  return width < NARROW_WINDOW_PX;
}

/** True while the window is narrower than `NARROW_WINDOW_PX`; follows resizes. */
export function useNarrowWindow(): boolean {
  const [narrow, setNarrow] = useState(() => typeof window !== "undefined" && isNarrowWindow(window.innerWidth));
  useEffect(() => {
    const onResize = () => setNarrow(isNarrowWindow(window.innerWidth));
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return narrow;
}

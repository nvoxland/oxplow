import { useEffect, useState } from "react";

const QUERY = "(pointer: coarse)";

const browserMatch = (query: string): MediaQueryList | null =>
  typeof globalThis.matchMedia === "function" ? globalThis.matchMedia(query) : null;

/** Whether the main pointer is a finger (a touch screen), following a change
 *  (a keyboard and trackpad attached to a tablet). What only a touch screen
 *  needs (the terminal's touch bar) is drawn while this is true. */
export function useCoarsePointer(match: (query: string) => MediaQueryList | null = browserMatch): boolean {
  const [coarse, setCoarse] = useState(() => match(QUERY)?.matches ?? false);
  useEffect(() => {
    const list = match(QUERY);
    if (!list) return;
    const update = () => setCoarse(list.matches);
    update();
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, [match]);
  return coarse;
}

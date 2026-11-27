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

/** Keep `root`'s `data-density` at `touch` while the main pointer is a
 *  finger, following a change, so the density tokens (`--control-height`,
 *  `--hit-min`, `apps/desktop/index.html`) are a finger's size there. A
 *  shell or a test that sets the attribute itself calls nothing. Returns a
 *  stop. */
export function followDensity(
  root: { setAttribute(name: string, value: string): void; removeAttribute(name: string): void },
  match: (query: string) => MediaQueryList | null = browserMatch,
): () => void {
  const list = match(QUERY);
  const update = () => {
    if (list?.matches) root.setAttribute("data-density", "touch");
    else root.removeAttribute("data-density");
  };
  update();
  list?.addEventListener("change", update);
  return () => list?.removeEventListener("change", update);
}

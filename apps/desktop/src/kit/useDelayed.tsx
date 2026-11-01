import { useEffect, useState, type ReactNode } from "react";

/** How long a load must last before its loading text shows. A load that
 *  answers sooner never flashes it. */
export const LOADING_AFTER_MS = 300;

/** `on`, once it has held for `ms`. */
export function useDelayed(on: boolean, ms: number = LOADING_AFTER_MS): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!on) {
      setShown(false);
      return;
    }
    const t = setTimeout(() => setShown(true), ms);
    return () => clearTimeout(t);
  }, [on, ms]);
  return on && shown;
}

/** Its children, only once it has been mounted for `ms`: the one way a
 *  loading line appears (`.context/usability.md`). */
export function DelayedLoading({ children, ms = LOADING_AFTER_MS }: { children: ReactNode; ms?: number }) {
  const shown = useDelayed(true, ms);
  return shown ? <>{children}</> : null;
}

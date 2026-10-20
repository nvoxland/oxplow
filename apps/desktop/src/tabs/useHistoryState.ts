import { useCallback, useRef, useState } from "react";

import { useOptionalPageNavigation } from "./PageNavigationContext.js";

/**
 * One value of a page's **history state** — this step of this tab's
 * history (`Entry.historyState`): it comes back with Back and Forward and
 * across a restart, as a browser's `history.state` does, and is never
 * shared. `key` names it within the entry; the value must be
 * JSON. Outside a tab, or in a body that isn't the tab's current page, it
 * is local state that starts from the entry's value.
 *
 * Setting what is already there touches nothing, so an effect may set it
 * freely.
 */
export function useHistoryState<T>(key: string, initial: T): [T, (value: T) => void] {
  const nav = useOptionalPageNavigation();
  const stored = nav?.historyState && key in nav.historyState ? (nav.historyState[key] as T) : undefined;
  const [local, setLocal] = useState<T>(stored ?? initial);
  const write = nav?.setHistoryState;
  const value = write ? (stored ?? initial) : local;
  const latest = useRef({ value, write });
  latest.current = { value, write };
  const set = useCallback(
    (next: T) => {
      const now = latest.current;
      if (JSON.stringify(next) === JSON.stringify(now.value)) return;
      if (now.write) now.write({ [key]: next });
      else setLocal(next);
    },
    [key],
  );
  return [value, set];
}

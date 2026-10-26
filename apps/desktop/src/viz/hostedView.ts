/**
 * The view a custom component shares with the page that hosts it
 * (`.context/extensions.md` → "Custom components", "Replacements"): its
 * view params and history state, what the bridge tells it (`init.view`)
 * and lets it set. Three hosts give one:
 * - a page that *is* the component: all its params, the whole entry;
 * - a replacement: the keys its target's contract lists
 *   (`UiReplacement.view`), as the core part it replaces reads them;
 * - a viz's own page: the viz's params.
 * Any other host (a dashboard tile, a slot, the rail) gives none, and the
 * component keeps its filters itself. A hosted component's history state
 * is its own object in the entry (`component:<scope>`), so it never
 * overwrites its page's.
 */
import { createContext, useMemo, useRef } from "react";

import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import type { ViewInit } from "./componentBridge.js";

export interface HostedView extends ViewInit {
  /** Change its view params (null drops one); throws `{ code: "INVALID" }`
   *  for a key not its to set. */
  setViewParams(params: Record<string, string | null>): void;
  setHistoryState(patch: Record<string, unknown>): void;
}

/** The history state key a hosted component keeps its own under. */
export const hostedStateKey = (scope: string) => `component:${scope}`;

const text = (v: SqlCell) => (v === null ? null : String(v));

export function useHostedView({
  keys,
  scope,
  write,
}: {
  /** The page's view params it shares; `"all"` for a page component. */
  keys: readonly string[] | "all";
  /** Whose history state it keeps (a replacement's target, `viz`); null
   *  for a page component, which has the whole entry. */
  scope: string | null;
  /** Write a change to the page's view; by default merged into the tab's
   *  entry (`patchViewParams`), a filter. A viz page writes by its
   *  params' rule. */
  write?(params: Record<string, string | null>): void;
}): HostedView | null {
  const nav = useOptionalPageNavigation();
  const navRef = useRef(nav);
  navRef.current = nav;
  const writeRef = useRef(write);
  writeRef.current = write;
  const inTab = !!nav?.setHistoryState;
  const keyList = keys === "all" ? "all" : keys.join(",");
  const params = nav?.viewParams;
  const shared = useMemo(() => {
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(params ?? {})) {
      if (keyList !== "all" && !keyList.split(",").includes(k)) continue;
      const t = text(v);
      if (t !== null) out[k] = t;
    }
    return out;
    // `params` is a fresh object per render; its content is the key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [JSON.stringify(params ?? {}), keyList]);
  const entry = nav?.historyState;
  const own = scope === null ? (entry ?? NONE) : ((entry?.[hostedStateKey(scope)] as Record<string, unknown> | undefined) ?? NONE);
  const ownKey = JSON.stringify(own);
  return useMemo(() => {
    if (!inTab || keyList === "") return null;
    return {
      viewParams: shared,
      historyState: own,
      setViewParams: (changes) => {
        if (keyList !== "all") {
          const allowed = keyList.split(",");
          const outside = Object.keys(changes).filter((k) => !allowed.includes(k));
          if (outside.length > 0) {
            const named = outside.map((k) => `\`${k}\``).join(", ");
            throw Object.assign(new Error(`${named} isn't in ${scope}'s view (${allowed.join(", ")}).`), { code: "INVALID" });
          }
        }
        if (writeRef.current) return writeRef.current(changes);
        navRef.current?.patchViewParams?.(Object.fromEntries(Object.entries(changes).map(([k, v]) => [k, v ?? undefined])));
      },
      setHistoryState: (patch) => {
        const set = navRef.current?.setHistoryState;
        if (!set) return;
        if (scope === null) return set(patch);
        const key = hostedStateKey(scope);
        const was = (navRef.current?.historyState?.[key] as Record<string, unknown> | undefined) ?? {};
        set({ [key]: { ...was, ...patch } });
      },
    };
    // `own` is read through its content key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inTab, keyList, scope, shared, ownKey]);
}

const NONE: Record<string, unknown> = {};

/** The view a page drawn inside another host gives its `custom` vizzes
 *  (a page replacement: the keys its target's contract lists); none: the
 *  page's components share no view. */
export const HostedViewContext = createContext<HostedView | null>(null);

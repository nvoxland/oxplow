/// The dock's state: which panel is open (at most one), an external store
/// read with `useDockState`. How tall the open panel is the device's own app
/// preference (`useDockHeight`).

import { useSyncExternalStore } from "react";

import { useAppPreference } from "../../prefs/remembered.js";

export const DOCK_MIN_HEIGHT = 160;
/** The panel never grows past its column's height minus this. */
export const DOCK_MAX_HEIGHT_MARGIN = 80;
const DEFAULT_HEIGHT = 280;

export interface DockState {
  open: string | null;
}

/** The height a remembered value stands for: the default for what isn't a
 *  positive number, else at least the minimum. */
export function dockHeightOf(raw: unknown): number {
  return typeof raw === "number" && Number.isFinite(raw) && raw > 0 ? Math.max(DOCK_MIN_HEIGHT, Math.round(raw)) : DEFAULT_HEIGHT;
}

/** How tall the open panel is, and the way to change it (clamped to the
 *  minimum; the maximum depends on the column, so the panel clamps the drag). */
export function useDockHeight(): [number, (px: number) => void] {
  const [raw, setRaw] = useAppPreference<number>("dockHeight", DEFAULT_HEIGHT);
  return [dockHeightOf(raw), (px) => setRaw(Math.max(DOCK_MIN_HEIGHT, Math.round(px)))];
}

export function createDockStore() {
  let state: DockState = { open: null };
  const listeners = new Set<() => void>();
  const set = (next: DockState) => {
    if (next.open === state.open) return;
    state = next;
    for (const l of listeners) l();
  };
  return {
    get: () => state,
    subscribe(l: () => void) {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    toggle: (id: string) => set({ ...state, open: state.open === id ? null : id }),
    open: (id: string) => set({ ...state, open: id }),
    close: () => set({ ...state, open: null }),
  };
}

export const dockStore = createDockStore();

export function useDockState(): DockState {
  return useSyncExternalStore(dockStore.subscribe, dockStore.get);
}

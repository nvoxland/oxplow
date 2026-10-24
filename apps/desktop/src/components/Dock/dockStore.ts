/// The dock's state: which panel is open (at most one) and how tall the
/// open panel is. An external store read with `useDockState`. The height
/// is remembered in the shell's storage (`shellStorage.ts`, which is total:
/// storage that refuses reads as nothing).

import { useSyncExternalStore } from "react";

import { shellStorage } from "../../shellStorage.js";

export const DOCK_MIN_HEIGHT = 160;
/** The panel never grows past its column's height minus this. */
export const DOCK_MAX_HEIGHT_MARGIN = 80;
const DEFAULT_HEIGHT = 280;
const HEIGHT_KEY = "oxplow.dock.height" as const;

export interface DockState {
  open: string | null;
  height: number;
}

function readHeight(): number {
  const n = Number(shellStorage.getItem(HEIGHT_KEY));
  return Number.isFinite(n) && n > 0 ? Math.max(DOCK_MIN_HEIGHT, Math.round(n)) : DEFAULT_HEIGHT;
}

export function createDockStore() {
  let state: DockState = { open: null, height: readHeight() };
  const listeners = new Set<() => void>();
  const set = (next: DockState) => {
    if (next.open === state.open && next.height === state.height) return;
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
    /** Clamp to the minimum and keep it. The maximum depends on the
     *  column, so the panel clamps the drag; this keeps what it is given. */
    setHeight(px: number) {
      const height = Math.max(DOCK_MIN_HEIGHT, Math.round(px));
      shellStorage.setItem(HEIGHT_KEY, String(height));
      set({ ...state, height });
    },
  };
}

export const dockStore = createDockStore();

export function useDockState(): DockState {
  return useSyncExternalStore(dockStore.subscribe, dockStore.get);
}

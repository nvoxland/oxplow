/// Which of Git ▸'s dialogs is open (`gitMenu.ts`): Compare with… for a
/// file, Rollback… over the stream's changes. Opened from any menu
/// (a tree row, the terminal, a link), drawn once by `GitDialogs` in the
/// window's tab area.
import { useSyncExternalStore } from "react";

export type GitDialog =
  | { kind: "compare"; path: string }
  | { kind: "rollback"; paths: string[]; checked: string[] }
  | null;

let open: GitDialog = null;
const listeners = new Set<() => void>();

function set(next: GitDialog): void {
  open = next;
  for (const l of listeners) l();
}

export const openCompareWith = (path: string): void => set({ kind: "compare", path });
export const openRollback = (paths: string[], checked: string[]): void => set({ kind: "rollback", paths, checked });
export const closeGitDialog = (): void => set(null);

/** The open dialog, re-rendering when it changes. */
export function useGitDialog(): GitDialog {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => open,
  );
}

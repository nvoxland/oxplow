import { useCallback, useRef } from "react";

import { useHistoryState } from "../tabs/useHistoryState.js";

/** The part of a Monaco editor (or a diff editor's modified side) that
 *  carries view state. */
export interface ViewStateEditor {
  saveViewState?(): unknown;
  restoreViewState?(state: never): void;
  onDidChangeCursorPosition(cb: () => void): { dispose(): void };
  onDidScrollChange(cb: () => void): { dispose(): void };
}

/**
 * Keeps Monaco's view state (cursor, scroll, folds, selection) with the
 * tab's entry, so Back and a restart land on the same line. `attach`
 * restores the saved blob and writes the next one back after `delayMs`
 * of quiet; it returns a detach that flushes a pending write.
 *
 * The blob is opaque and JSON-safe (`saveViewState`). `key` names it in the
 * entry's history state.
 */
export function useMonacoViewState(key: string, delayMs = 300): (editor: ViewStateEditor, events?: ViewStateEditor) => () => void {
  const [saved, setSaved] = useHistoryState<unknown>(key, null);
  const latest = useRef({ saved, setSaved });
  latest.current = { saved, setSaved };
  return useCallback(
    (editor, events = editor) => {
      const state = latest.current.saved;
      if (state) {
        try {
          editor.restoreViewState?.(state as never);
        } catch {
          /* a state from another model: ignore it */
        }
      }
      let timer: ReturnType<typeof setTimeout> | null = null;
      const save = () => {
        timer = null;
        const next = editor.saveViewState?.() ?? null;
        if (next) latest.current.setSaved(next);
      };
      const schedule = () => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(save, delayMs);
      };
      const disposers = [events.onDidChangeCursorPosition(schedule), events.onDidScrollChange(schedule)];
      return () => {
        disposers.forEach((d) => d.dispose());
        if (timer) {
          clearTimeout(timer);
          save();
        }
      };
    },
    [delayMs],
  );
}

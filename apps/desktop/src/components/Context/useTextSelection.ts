// The fresh text selection in a page's body, for the Add to Context
// toolbar (`SelectionContextToolbar`).
//
// On `mouseup` the window's selection is read once and kept (a press on
// the toolbar would collapse the live one). It counts only when it is not
// empty, starts inside `root`, and is outside an editor or a field: text
// in an input, a textarea, a contenteditable (the markdown editor) or
// Monaco is that surface's own, with its own menu — the same carve-out as
// the native context menu (`context-menu.ts`). Escape forgets it.

import { useCallback, useEffect, useState } from "react";

import { describeChain, shouldSuppressContextMenu } from "../../context-menu.js";

/** The attribute on the toolbar's own elements: a mouseup there is not a
 *  new selection. */
export const SELECTION_UI_ATTR = "data-selection-context-ui";

/** A selection, kept at the time it was made. */
export interface TextSelection {
  /** The selected text, trimmed. */
  text: string;
  /** Where it is in the viewport: what the toolbar floats over. */
  rect: DOMRect;
}

/** The window's selection, when it is page text inside `root`; else null. */
export function readSelection(root: Element): TextSelection | null {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
  const text = sel.toString().trim();
  if (!text) return null;
  const range = sel.getRangeAt(0);
  if (!root.contains(range.startContainer)) return null;
  if (!shouldSuppressContextMenu(describeChain(range.startContainer))) return null;
  return { text, rect: range.getBoundingClientRect() };
}

/** The selection in `root`'s text the person just made, and a way to
 *  forget it. */
export function useTextSelection(root: Element | null): { selection: TextSelection | null; clear: () => void } {
  const [selection, setSelection] = useState<TextSelection | null>(null);

  useEffect(() => {
    if (!root) return;
    const onMouseUp = (e: MouseEvent) => {
      if ((e.target as Element | null)?.closest?.(`[${SELECTION_UI_ATTR}]`)) return;
      // A tick later: the browser finishes the selection after mouseup.
      window.setTimeout(() => setSelection(readSelection(root)), 0);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSelection(null);
    };
    document.addEventListener("mouseup", onMouseUp);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mouseup", onMouseUp);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [root]);

  const clear = useCallback(() => setSelection(null), []);
  return { selection, clear };
}

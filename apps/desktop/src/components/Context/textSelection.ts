// The selected text in a page's body, for the page's right-click Add
// Selection to Context (`SelectionContextMenu`).
//
// It counts only when it is not empty, starts inside `root`, and is outside
// an editor or a field: text in an input, a textarea, a contenteditable (the
// markdown editor) or Monaco is that surface's own, with its own menu — the
// same carve-out as the native context menu (`context-menu.ts`). So is a
// terminal's: xterm draws its rows as text, but what the browser selects
// there is not what the person highlighted; the terminal's own menu adds
// that (`terminal/terminalMenu.ts`).

import { describeChain, shouldSuppressContextMenu } from "../../context-menu.js";

/** A page's selected text, and the range it covers. */
export interface TextSelection {
  /** The selected text, trimmed. */
  text: string;
  range: Range;
}

/** The window's selection, when it is page text inside `root`; else null. */
export function readSelection(root: Element): TextSelection | null {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
  const text = sel.toString().trim();
  if (!text) return null;
  const range = sel.getRangeAt(0);
  if (!root.contains(range.startContainer)) return null;
  const chain = describeChain(range.startContainer);
  if (!shouldSuppressContextMenu(chain) || chain.some((n) => n.classes.includes("xterm"))) return null;
  return { text, range };
}

/// A page's right-click menu over its selected text: Copy, and Add
/// Selection to Context (the text on the page's URI; `addToContext` opens
/// the Context panel's add card). As the editor's, the diff's and the terminal's menus
/// offer it; nothing floats over a selection. It answers a right-click
/// only on the selection itself, and before a row's own menu: it is React's
/// capture handler on the page's body, and React runs those from the
/// outside in (a row opens its menu in its own capture handler, so a native
/// listener on the body would run after it). Anywhere else the right-click
/// is the row's, or nobody's.
import { useCallback, type MouseEvent, type ReactNode } from "react";

import { addToContext } from "../../contextBasket.js";
import { useContextMenu } from "../useRowContextMenu.js";
import { readSelection } from "./textSelection.js";
import { copyText } from "../../clipboard.js";

export function useSelectionContextMenu(uri: string | null): {
  /** The body's `onContextMenuCapture`. */
  onContextMenuCapture(e: MouseEvent<HTMLElement>): void;
  menu: ReactNode;
} {
  const menu = useContextMenu();
  const { open } = menu;
  const onContextMenuCapture = useCallback(
    (e: MouseEvent<HTMLElement>) => {
      const selection = readSelection(e.currentTarget);
      if (!selection || !(e.target instanceof Node) || !selection.range.intersectsNode(e.target)) return;
      const { text } = selection;
      open(e, [
        { id: "copy", label: "Copy", enabled: true, run: () => void copyText(text) },
        { id: "add-to-context", label: "Add Selection to Context", enabled: true, icon: "at-sign", run: () => void addToContext({ uri, text }) },
      ]);
    },
    [uri, open],
  );
  return { onContextMenuCapture, menu: menu.menu };
}

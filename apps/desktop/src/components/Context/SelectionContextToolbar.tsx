import { AtSign } from "lucide-react";

import { addToContext } from "../../contextBasket.js";
import { SELECTION_UI_ATTR, useTextSelection } from "./useTextSelection.js";

/// A small floating "Add to Context" button over a fresh text selection in
/// a page's body (`root`, mounted by `PageHost`). A press puts the selected
/// text in the thread's basket on the page's URI (`uri`); `addToContext`
/// opens the note prompt. It never takes the right-click, and goes on a new
/// selection, a press or Escape. Editors and fields keep their own menus
/// (`useTextSelection`).
export function SelectionContextToolbar({ root, uri }: { root: Element | null; uri: string | null }) {
  const { selection, clear } = useTextSelection(root);
  if (!selection) return null;
  const { rect, text } = selection;
  return (
    <SelectionContextButton
      left={rect.left}
      top={rect.top}
      onAdd={() => {
        void addToContext({ uri, text });
        clear();
      }}
    />
  );
}

/// The floating "Add to Context" button itself, above (`left`, `top`) in
/// the viewport, kept on screen; the page's toolbar and the terminal's
/// own selection (`TerminalSurface`) both float it.
export function SelectionContextButton({ left, top, onAdd }: { left: number; top: number; onAdd: () => void }) {
  return (
    <div {...{ [SELECTION_UI_ATTR]: "" }} style={{ position: "fixed", left: Math.min(Math.max(8, left), window.innerWidth - 160), top: Math.max(8, top - 38), zIndex: 1000 }}>
      <button
        type="button"
        {...{ [SELECTION_UI_ATTR]: "" }}
        data-testid="selection-context-button"
        // On the press: a click would collapse the selection first.
        onMouseDown={(e) => {
          e.preventDefault();
          e.stopPropagation();
          onAdd();
        }}
        style={buttonStyle}
      >
        <AtSign size={14} aria-hidden />
        Add to Context
      </button>
    </div>
  );
}

const buttonStyle = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  padding: "5px 10px",
  fontFamily: "var(--font-ui)",
  fontSize: 12,
  color: "var(--text-primary)",
  background: "var(--surface-elevated)",
  border: "1px solid var(--border-strong)",
  borderRadius: 6,
  boxShadow: "0 4px 14px rgba(0,0,0,0.35)",
  cursor: "pointer",
} as const;

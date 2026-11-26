/// Selecting by touch in the code editor, under it on a touch screen: Monaco
/// selects a word on a double tap and nothing more, with no handle to
/// stretch it. So: Expand and Shrink (its smart selection: the word, the
/// expression, the line, the block, the file), Select to here (the next tap
/// stretches the selection from where it starts to the tapped spot), Line,
/// All, Copy and Paste. Presses keep the editor's focus, so the on-screen
/// keyboard stays where it is.
import { useEffect, useState, type SyntheticEvent } from "react";

import { copyText, readText } from "../clipboard.js";
import { touchBarStyle, touchKeyStyle, touchRowStyle } from "../touchBarStyles.js";

type Position = { lineNumber: number; column: number };
type Selection = {
  startLineNumber: number;
  startColumn: number;
  endLineNumber: number;
  endColumn: number;
  /** Where the selection was started (its anchor), and where it ends now. */
  getStartPosition?(): Position;
  getPosition?(): Position;
};

/** What the bar needs of the editor (Monaco's `ICodeEditor`). */
export interface EditorLike {
  getSelection(): Selection | null;
  setSelection(range: { startLineNumber: number; startColumn: number; endLineNumber: number; endColumn: number }): void;
  getModel(): { getValueInRange(range: Selection): string } | null;
  getAction(id: string): { run(): unknown } | null;
  executeEdits(source: string, edits: Array<{ range: Selection; text: string; forceMoveMarkers?: boolean }>): unknown;
  onDidChangeCursorPosition(listener: (event: { position: Position; source: string }) => void): { dispose(): void };
  focus(): void;
}

const keepFocus = (event: SyntheticEvent) => event.preventDefault();

/** The selection from `anchor` to `to`, either way round. Pure. */
export function stretch(anchor: Position, to: Position) {
  const before = to.lineNumber < anchor.lineNumber || (to.lineNumber === anchor.lineNumber && to.column < anchor.column);
  const [a, b] = before ? [to, anchor] : [anchor, to];
  return { startLineNumber: a.lineNumber, startColumn: a.column, endLineNumber: b.lineNumber, endColumn: b.column };
}

export function EditorTouchBar({
  editor,
  copy = copyText,
  read = readText,
}: {
  editor: EditorLike | null;
  copy?: (text: string) => Promise<boolean>;
  read?: () => Promise<string | null>;
}) {
  // Armed: where the selection starts, waiting for the tap that ends it.
  const [anchor, setAnchor] = useState<Position | null>(null);

  useEffect(() => {
    if (!editor || anchor === null) return;
    const sub = editor.onDidChangeCursorPosition((event) => {
      // The tap (Monaco says "mouse" for a touch too); a key leaves it armed.
      if (event.source !== "mouse") return;
      editor.setSelection(stretch(anchor, event.position));
      setAnchor(null);
    });
    return () => sub.dispose();
  }, [editor, anchor]);

  if (!editor) return null;
  const run = (id: string) => {
    void editor.getAction(id)?.run();
    editor.focus();
  };
  const selected = () => {
    const selection = editor.getSelection();
    return selection ? (editor.getModel()?.getValueInRange(selection) ?? "") : "";
  };
  const key = (id: string, label: string, onPress: () => void, on = false) => (
    <button
      key={id}
      type="button"
      data-testid={`editor-touch-${id}`}
      onPointerDown={keepFocus}
      onMouseDown={keepFocus}
      onClick={onPress}
      aria-pressed={on || undefined}
      style={touchKeyStyle(on)}
    >
      {label}
    </button>
  );
  return (
    <div data-testid="editor-touch-bar" style={touchBarStyle}>
      <div style={touchRowStyle}>
        {key("expand", "Expand", () => run("editor.action.smartSelect.expand"))}
        {key("shrink", "Shrink", () => run("editor.action.smartSelect.shrink"))}
        {key(
          "to-here",
          anchor ? "Tap the end…" : "Select to here",
          () => {
            if (anchor) {
              setAnchor(null);
              return;
            }
            const selection = editor.getSelection();
            const start = selection?.getStartPosition?.() ?? (selection ? { lineNumber: selection.startLineNumber, column: selection.startColumn } : null);
            if (start) setAnchor(start);
          },
          anchor !== null,
        )}
        {key("line", "Line", () => run("expandLineSelection"))}
        {key("all", "All", () => run("editor.action.selectAll"))}
        {key("copy", "Copy", () => {
          const text = selected();
          if (text) void copy(text);
        })}
        {key("paste", "Paste", () => {
          void read().then((text) => {
            const selection = editor.getSelection();
            if (text === null || !selection) return;
            editor.executeEdits("touch-paste", [{ range: selection, text, forceMoveMarkers: true }]);
            editor.focus();
          });
        })}
      </div>
    </div>
  );
}

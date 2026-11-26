import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { EditorTouchBar, stretch, type EditorLike } from "./EditorTouchBar.js";

afterEach(() => cleanup());

type Pos = { lineNumber: number; column: number };

function fakeEditor(text = "hello world") {
  const calls: string[] = [];
  let selection = { startLineNumber: 1, startColumn: 1, endLineNumber: 1, endColumn: 6 };
  let listener: ((e: { position: Pos; source: string }) => void) | null = null;
  const editor: EditorLike = {
    getSelection: () => ({ ...selection, getStartPosition: () => ({ lineNumber: selection.startLineNumber, column: selection.startColumn }) }),
    setSelection: (r) => {
      selection = r;
      calls.push(`select ${r.startLineNumber}:${r.startColumn}-${r.endLineNumber}:${r.endColumn}`);
    },
    getModel: () => ({ getValueInRange: (r) => text.slice(r.startColumn - 1, r.endColumn - 1) }),
    getAction: (id) => ({ run: () => calls.push(`action ${id}`) }),
    trigger: (_s, id, payload) => calls.push(`${id} ${(payload as { text: string }).text}`),
    onDidChangeCursorPosition: (l) => {
      listener = l;
      return { dispose: () => (listener = null) };
    },
    focus: () => {},
  };
  return { editor, calls, tap: (p: Pos, source = "mouse") => listener?.({ position: p, source }) };
}

test("a selection stretches either way from where it starts", () => {
  expect(stretch({ lineNumber: 2, column: 3 }, { lineNumber: 4, column: 1 })).toEqual({
    startLineNumber: 2, startColumn: 3, endLineNumber: 4, endColumn: 1,
  });
  expect(stretch({ lineNumber: 2, column: 3 }, { lineNumber: 1, column: 5 })).toEqual({
    startLineNumber: 1, startColumn: 5, endLineNumber: 2, endColumn: 3,
  });
});

// Expand, Shrink, Line and All are the editor's own selection actions.
test("the keys run the editor's selection actions", () => {
  const { editor, calls } = fakeEditor();
  render(<EditorTouchBar editor={editor} />);
  for (const id of ["expand", "shrink", "line", "all"]) fireEvent.click(screen.getByTestId(`editor-touch-${id}`));
  expect(calls).toEqual([
    "action editor.action.smartSelect.expand",
    "action editor.action.smartSelect.shrink",
    "action expandLineSelection",
    "action editor.action.selectAll",
  ]);
});

// Select to here waits for a tap; a key moving the cursor does not end it.
test("select to here stretches the selection to the next tap", () => {
  const { editor, calls, tap } = fakeEditor();
  render(<EditorTouchBar editor={editor} />);
  fireEvent.click(screen.getByTestId("editor-touch-to-here"));
  expect(screen.getByTestId("editor-touch-to-here").textContent).toBe("Tap the end…");
  act(() => tap({ lineNumber: 1, column: 9 }, "keyboard"));
  expect(calls).toEqual([]);
  act(() => tap({ lineNumber: 3, column: 2 }));
  expect(calls).toEqual(["select 1:1-3:2"]);
  expect(screen.getByTestId("editor-touch-to-here").textContent).toBe("Select to here");
});

test("copy takes the selected text and paste puts the clipboard over it", async () => {
  const { editor, calls } = fakeEditor();
  const copied: string[] = [];
  render(
    <EditorTouchBar
      editor={editor}
      copy={async (t) => {
        copied.push(t);
        return true;
      }}
      read={async () => "pasted"}
    />,
  );
  fireEvent.click(screen.getByTestId("editor-touch-copy"));
  expect(copied).toEqual(["hello"]);
  fireEvent.click(screen.getByTestId("editor-touch-paste"));
  await waitFor(() => expect(calls).toContain("paste pasted"));
});

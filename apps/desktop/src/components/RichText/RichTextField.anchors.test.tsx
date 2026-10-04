import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

// tsk902: a comment's re-found anchor is stored when the document settles
// — at once when nobody is typing, on blur when someone is — never per
// typing pause, since each store is a command run on the person's behalf.

const realApi = await import("../../api.js");
const relocated: Array<[string, boolean]> = [];
const comment = {
  id: "cmt1",
  quote: "brown fox",
  // A stale hint: the quote isn't at 0..9.
  selectors_json: JSON.stringify({ from: 0, to: 9 }),
  orphaned: false,
};
mock.module("../../api.js", () => ({
  ...realApi,
  listCommentsForTarget: async () => [{ comment, replies: [] }],
  subscribeCommentEvents: () => () => {},
  onRemoteReconnect: () => () => {},
  relocateComment: async (id: string, _anchor: string, orphaned: boolean) => {
    relocated.push([id, orphaned]);
  },
}));

const { RichTextField } = await import("./RichTextField.js");

afterEach(() => {
  cleanup();
  relocated.length = 0;
});

const config = { streamId: "str1", threadId: null, targetKind: "wiki", targetId: "page" };
const settle = () => act(async () => new Promise((r) => setTimeout(r, 250)));

test("a settled document stores a moved anchor once", async () => {
  render(<RichTextField value="The quick brown fox" onCommit={() => {}} comments={config} />);
  await settle();
  expect(relocated).toEqual([["cmt1", false]]);
});

test("while the person types, a moved anchor waits for blur", async () => {
  const view = render(<RichTextField value="The quick brown fox" onCommit={() => {}} comments={config} />);
  const editable = view.container.querySelector(".ProseMirror") as HTMLElement & {
    editor?: {
      isFocused: boolean;
      commands: { focus(pos?: string): boolean; insertContentAt(pos: number, text: string): boolean; blur(): boolean };
    };
  };
  await settle();
  const editor = editable.editor;
  expect(editor).toBeDefined();
  await act(async () => {
    editor?.commands.focus("start");
  });
  relocated.length = 0;
  // Typing above the quote moves it; while focused nothing is stored.
  for (const text of ["A ", "B ", "C "]) {
    await act(async () => {
      editor?.commands.insertContentAt(1, text);
    });
    await settle();
  }
  const whileTyping = relocated.length;
  await act(async () => {
    editor?.commands.blur();
  });
  await settle();
  expect(whileTyping).toBe(0);
  expect(relocated).toEqual([["cmt1", false]]);
});

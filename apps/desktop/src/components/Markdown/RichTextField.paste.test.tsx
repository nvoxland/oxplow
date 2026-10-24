import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { Editor } from "@tiptap/core";

import { fileUri, wikiUri } from "../../uri/uri.js";
import { getMarkdown } from "./richTextExtensions.js";
import { insertRefLinks } from "./RichTextField.js";
import { RichTextField } from "./RichTextField.js";

// A pasted oxplow URI goes in as its link; a URI or a web URL pasted over
// a selection links the selection; dropped refs go in as links, a space
// between.

afterEach(cleanup);

async function editorOf(value: string): Promise<Editor> {
  const view = render(<RichTextField value={value} onChange={() => {}} />);
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  return (view.container.querySelector(".ProseMirror") as unknown as { editor: Editor }).editor;
}

function paste(editor: Editor, text: string) {
  fireEvent.paste(editor.view.dom, { clipboardData: { types: ["text/plain"], getData: (t: string) => (t === "text/plain" ? text : "") } });
}

test("a pasted URI goes in as its link", async () => {
  const editor = await editorOf("See");
  act(() => {
    editor.commands.focus("end");
    editor.commands.insertContent(" ");
    paste(editor, wikiUri("notes"));
  });
  expect(getMarkdown(editor)).toBe(`See [notes](${wikiUri("notes")})`);
});

test("a web URL pasted over a selection links it", async () => {
  const editor = await editorOf("Hello world");
  act(() => {
    editor.chain().focus().setTextSelection({ from: 1, to: 6 }).run();
    paste(editor, "https://example.com");
  });
  expect(getMarkdown(editor)).toBe("[Hello](https://example.com) world");
});

test("plain text pastes as text", async () => {
  const editor = await editorOf("Hello");
  act(() => {
    editor.commands.focus("end");
    paste(editor, " there");
  });
  expect(getMarkdown(editor)).toBe("Hello there");
});

test("dropped refs go in as links, a space between", async () => {
  const editor = await editorOf("See");
  act(() => {
    insertRefLinks(editor, editor.state.doc.content.size - 1, [
      { uri: wikiUri("notes"), title: "notes" },
      { uri: fileUri("src/a.ts"), title: "src/a.ts" },
    ], () => null);
  });
  expect(getMarkdown(editor)).toBe(`See [notes](${wikiUri("notes")}) [src/a.ts](${fileUri("src/a.ts")})`);
});

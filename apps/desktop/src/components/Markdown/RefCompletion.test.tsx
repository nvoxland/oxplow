import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { Editor } from "@tiptap/core";

import { wikiUri } from "../../uri/uri.js";
import { getMarkdown } from "./richTextExtensions.js";
import type { RefOption } from "./refOptions.js";
import { RichTextField } from "./RichTextField.js";

// Typing `[[` lists what it could link to, filtered as the person types;
// Enter puts the picked one in as its link (saved as the bare wikilink),
// Escape closes the list and keeps what was typed.

afterEach(cleanup);

const OPTIONS: RefOption[] = [
  { uri: wikiUri("architecture"), title: "Architecture", kind: "wiki" },
  { uri: wikiUri("arch-notes"), title: "Arch notes", kind: "wiki" },
];
const queries: string[] = [];
const search = async (query: string) => {
  queries.push(query);
  return OPTIONS.filter((o) => o.title.toLowerCase().includes(query.toLowerCase()));
};

async function editorOf(view: ReturnType<typeof render>): Promise<Editor> {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  return (view.container.querySelector(".ProseMirror") as unknown as { editor: Editor }).editor;
}

const key = (editor: Editor, k: string) => fireEvent.keyDown(editor.view.dom, { key: k });

test("[[ lists the matches and Enter puts the first in as its link", async () => {
  const view = render(<RichTextField value="See" onChange={() => {}} refSearch={search} />);
  const editor = await editorOf(view);
  act(() => {
    editor.chain().focus("end").insertContent(" [[arch").run();
  });
  await waitFor(() => expect(document.querySelectorAll("[data-testid^='ref-completion-row-']").length).toBe(2));
  expect(queries.at(-1)).toBe("arch");
  act(() => {
    key(editor, "ArrowDown");
    key(editor, "ArrowUp");
    key(editor, "Enter");
  });
  await waitFor(() => expect(document.querySelector("[data-testid='ref-completion']")).toBeNull());
  expect(getMarkdown(editor)).toBe(`See [architecture](${wikiUri("architecture")}) `);
});

test("Escape closes the list and keeps the typed text", async () => {
  const escapes: number[] = [];
  const view = render(<RichTextField value="" onChange={() => {}} onEscape={() => escapes.push(1)} refSearch={search} />);
  const editor = await editorOf(view);
  act(() => {
    editor.chain().focus("end").insertContent("[[arch").run();
  });
  await waitFor(() => expect(document.querySelector("[data-testid='ref-completion']")).not.toBeNull());
  act(() => {
    key(editor, "Escape");
  });
  await waitFor(() => expect(document.querySelector("[data-testid='ref-completion']")).toBeNull());
  // The list's Escape, not the owner's.
  expect(escapes).toEqual([]);
  expect(getMarkdown(editor)).toContain("arch");
});

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { Editor } from "@tiptap/core";

import { wikiUri } from "../../uri/uri.js";
import { getMarkdown } from "./richTextExtensions.js";
import { RichTextField } from "./RichTextField.js";
import { selectionToolbarVisible } from "./SelectionToolbar.js";

// A selection in the editor floats the format toolbar: Bold, Italic, Code,
// Link and the headings (not in a one-line field); Add comment only where
// the text takes comments.

afterEach(cleanup);

async function selectHello(view: ReturnType<typeof render>): Promise<Editor> {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  const editor = (view.container.querySelector(".ProseMirror") as unknown as { editor: Editor }).editor;
  act(() => {
    editor.chain().focus().setTextSelection({ from: 1, to: 6 }).run();
  });
  await waitFor(() => expect(document.querySelector("[data-testid='selection-toolbar']")).not.toBeNull());
  return editor;
}

const press = (id: string) => fireEvent.mouseDown(document.querySelector(`[data-testid='selection-toolbar-${id}']`)!);

test("Bold and Italic format the selection, and say they are on", async () => {
  const view = render(<RichTextField value="Hello world" onChange={() => {}} />);
  const editor = await selectHello(view);
  act(() => press("bold"));
  expect(getMarkdown(editor)).toBe("**Hello** world");
  await waitFor(() => expect(document.querySelector("[data-testid='selection-toolbar-bold']")?.getAttribute("aria-pressed")).toBe("true"));
  act(() => press("italic"));
  expect(getMarkdown(editor)).toBe("***Hello*** world");
});

test("Link takes a web URL, a wikilink, and nothing to remove it", async () => {
  const view = render(<RichTextField value="Hello world" onChange={() => {}} />);
  const editor = await selectHello(view);
  const link = (value: string) => {
    act(() => press("link"));
    const input = document.querySelector("[data-testid='selection-toolbar-link-input']") as HTMLInputElement;
    fireEvent.change(input, { target: { value } });
    fireEvent.keyDown(input, { key: "Enter" });
  };
  link("https://example.com");
  expect(getMarkdown(editor)).toBe("[Hello](https://example.com) world");
  act(() => {
    editor.chain().focus().setTextSelection({ from: 1, to: 6 }).run();
  });
  link("local-snapshots");
  expect(getMarkdown(editor)).toBe(`[Hello](${wikiUri("local-snapshots")}) world`);
  act(() => {
    editor.chain().focus().setTextSelection({ from: 1, to: 6 }).run();
  });
  link("");
  expect(getMarkdown(editor)).toBe("Hello world");
});

test("Escape in the link box leaves the text as it was", async () => {
  const view = render(<RichTextField value="Hello world" onChange={() => {}} />);
  const editor = await selectHello(view);
  act(() => press("link"));
  const input = document.querySelector("[data-testid='selection-toolbar-link-input']") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "https://example.com" } });
  fireEvent.keyDown(input, { key: "Escape" });
  expect(document.querySelector("[data-testid='selection-toolbar-link-input']")).toBeNull();
  expect(getMarkdown(editor)).toBe("Hello world");
});

test("headings only where blocks are; Add comment only where comments are", async () => {
  const view = render(<RichTextField value="Hello world" onChange={() => {}} />);
  const editor = await selectHello(view);
  act(() => press("h2"));
  expect(getMarkdown(editor)).toBe("## Hello world");
  expect(document.querySelector("[data-testid='selection-toolbar-comment']")).toBeNull();
  cleanup();
  const one = render(<RichTextField value="Hello world" onChange={() => {}} inlineOnly />);
  await selectHello(one);
  expect(document.querySelector("[data-testid='selection-toolbar-h2']")).toBeNull();
  expect(document.querySelector("[data-testid='selection-toolbar-bold']")).not.toBeNull();
});

describe("selectionToolbarVisible", () => {
  const base = {
    selectionEmpty: false,
    composerOpen: false,
    popoverOpen: false,
  };

  test("shows for a non-empty selection", () => {
    expect(selectionToolbarVisible(base)).toBe(true);
  });

  test("hidden for a collapsed selection", () => {
    expect(selectionToolbarVisible({ ...base, selectionEmpty: true })).toBe(false);
  });

  test("hidden while the composer or a thread popover is open", () => {
    expect(selectionToolbarVisible({ ...base, composerOpen: true })).toBe(false);
    expect(selectionToolbarVisible({ ...base, popoverOpen: true })).toBe(false);
  });
});

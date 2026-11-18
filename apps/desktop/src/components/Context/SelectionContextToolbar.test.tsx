import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { useState } from "react";

import type { ContextWhat } from "../../contextBasket.js";

// A fresh text selection in a page's body floats Add to Context over it; a
// press puts the selected text in the basket on the page's URI (the note
// prompt it opens is `addToContext`'s). Text in an editor or a field is
// that surface's own: no toolbar there.

const added: ContextWhat[] = [];
const realBasket = { ...(await import("../../contextBasket.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../../contextBasket.js", () => ({
  ...realBasket,
  addToContext: async (what: ContextWhat) => {
    added.push(what);
  },
}));
const { SelectionContextToolbar } = await import("./SelectionContextToolbar.js");

const URI = "oxplow:oxplow/knowledge?id=notes&store=oxwiki";

afterEach(() => {
  cleanup();
  added.length = 0;
  window.getSelection()?.removeAllRanges();
});

/** A page body with the toolbar over it, as `PageHost` mounts it. */
function Page({ children }: { children: React.ReactNode }) {
  const [root, setRoot] = useState<HTMLDivElement | null>(null);
  return (
    <div>
      <div data-testid="body" ref={setRoot}>
        {children}
      </div>
      <p data-testid="outside">Outside text</p>
      <SelectionContextToolbar root={root} uri={URI} />
    </div>
  );
}

function setup(content: React.ReactNode) {
  return render(<Page>{content}</Page>);
}

async function select(el: Element) {
  const range = document.createRange();
  range.selectNodeContents(el);
  const sel = window.getSelection()!;
  sel.removeAllRanges();
  sel.addRange(range);
  await act(async () => {
    fireEvent.mouseUp(el);
    await new Promise((r) => setTimeout(r, 5));
  });
}

test("a selection in the page's text floats Add to Context, which adds the text on the page's URI", async () => {
  const view = setup(<p data-testid="para">The quick brown fox</p>);
  await select(view.getByTestId("para"));
  const button = await waitFor(() => view.getByTestId("selection-context-button"));
  expect(button.textContent).toContain("Add to Context");
  fireEvent.mouseDown(button);
  expect(added).toEqual([{ uri: URI, text: "The quick brown fox" }]);
  // Pressed, the toolbar goes.
  await waitFor(() => expect(view.queryByTestId("selection-context-button")).toBeNull());
});

test("a selection that collapses, or Escape, hides it", async () => {
  const view = setup(<p data-testid="para">The quick brown fox</p>);
  await select(view.getByTestId("para"));
  await waitFor(() => view.getByTestId("selection-context-button"));
  act(() => {
    fireEvent.keyDown(document, { key: "Escape" });
  });
  await waitFor(() => expect(view.queryByTestId("selection-context-button")).toBeNull());
  await select(view.getByTestId("para"));
  await waitFor(() => view.getByTestId("selection-context-button"));
  window.getSelection()!.removeAllRanges();
  await act(async () => {
    fireEvent.mouseUp(view.getByTestId("para"));
    await new Promise((r) => setTimeout(r, 5));
  });
  expect(view.queryByTestId("selection-context-button")).toBeNull();
});

test("no toolbar over an editor's text, or text outside the page", async () => {
  const view = setup(
    <div contentEditable suppressContentEditableWarning data-testid="editable">
      Typed text
    </div>,
  );
  await select(view.getByTestId("editable"));
  expect(view.queryByTestId("selection-context-button")).toBeNull();
  await select(view.getByTestId("outside"));
  expect(view.queryByTestId("selection-context-button")).toBeNull();
});

// A terminal keeps its own selection (xterm draws rows as text, but what
// the browser selects there is not what the person highlighted): its
// right-click menu adds it, never this toolbar.
test("no toolbar over a terminal's rows", async () => {
  const view = setup(
    <div className="xterm">
      <span data-testid="row">note</span>
    </div>,
  );
  await select(view.getByTestId("row"));
  expect(view.queryByTestId("selection-context-button")).toBeNull();
});

// `mock.module` outlives this file: put the real module back for the files that run after it.
afterAll(() => {
  mock.module("../../contextBasket.js", () => realBasket);
});

import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { ContextWhat } from "../../contextBasket.js";

// A right-click on a page's selected text opens a menu of Copy and Add
// Selection to Context: the text goes in the basket on the page's URI (the
// note prompt it opens is `addToContext`'s). Nothing floats over a
// selection. A right-click off the selection, or over an editor's, a
// field's or a terminal's text, is not this menu's.

const added: ContextWhat[] = [];
const realBasket = { ...(await import("../../contextBasket.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../../contextBasket.js", () => ({
  ...realBasket,
  addToContext: async (what: ContextWhat) => {
    added.push(what);
  },
}));
const { useSelectionContextMenu } = await import("./SelectionContextMenu.js");

const URI = "oxplow:oxplow/knowledge?id=notes&store=oxwiki";

afterEach(() => {
  cleanup();
  added.length = 0;
  window.getSelection()?.removeAllRanges();
});

/** A page body with the menu on it, as `PageHost` mounts it. */
function Page({ children }: { children: React.ReactNode }) {
  const { onContextMenuCapture, menu } = useSelectionContextMenu(URI);
  return (
    <div>
      <div data-testid="body" onContextMenuCapture={onContextMenuCapture}>
        {children}
      </div>
      {menu}
    </div>
  );
}

function select(el: Element) {
  const range = document.createRange();
  range.selectNodeContents(el);
  const sel = window.getSelection()!;
  sel.removeAllRanges();
  sel.addRange(range);
}

test("a right-click on the selected text offers Copy and Add Selection to Context", async () => {
  const view = render(
    <Page>
      <p data-testid="para">The quick brown fox</p>
    </Page>,
  );
  select(view.getByTestId("para"));
  // No button floats over it.
  await act(async () => {
    fireEvent.mouseUp(view.getByTestId("para"));
    await new Promise((r) => setTimeout(r, 5));
  });
  expect(view.queryByTestId("selection-context-button")).toBeNull();
  const event = fireEvent.contextMenu(view.getByTestId("para"));
  expect(event).toBe(false); // the OS menu and a row's menu don't open
  const items = await waitFor(() => [view.getByTestId("menu-item-copy"), view.getByTestId("menu-item-add-to-context")]);
  expect(items.map((i) => i.textContent?.trim())).toEqual(["Copy", "Add Selection to Context"]);
  fireEvent.click(view.getByTestId("menu-item-add-to-context"));
  await waitFor(() => expect(added).toEqual([{ uri: URI, text: "The quick brown fox" }]));
});

test("a right-click off the selection, or with none, is not this menu's", async () => {
  const view = render(
    <Page>
      <p data-testid="para">The quick brown fox</p>
      <p data-testid="other">Another line</p>
    </Page>,
  );
  expect(fireEvent.contextMenu(view.getByTestId("para"))).toBe(true);
  select(view.getByTestId("para"));
  expect(fireEvent.contextMenu(view.getByTestId("other"))).toBe(true);
  expect(view.queryByTestId("menu-item-add-to-context")).toBeNull();
});

// A row opens its own menu in its capture phase: the selection's menu
// still comes first (the row's is what a right-click off the selection gets).
test("on a row's selected text the selection's menu comes before the row's", async () => {
  const rowMenu: string[] = [];
  const view = render(
    <Page>
      <div data-testid="row" onContextMenuCapture={() => rowMenu.push("row")}>
        <span data-testid="cell">Selected cell</span>
      </div>
      <span data-testid="other">Elsewhere</span>
    </Page>,
  );
  select(view.getByTestId("cell"));
  fireEvent.contextMenu(view.getByTestId("cell"));
  await waitFor(() => view.getByTestId("menu-item-add-to-context"));
  expect(rowMenu).toEqual([]);
});

test("an editor's, a field's or a terminal's selected text keeps its own menu", async () => {
  const view = render(
    <Page>
      <div contentEditable suppressContentEditableWarning data-testid="editable">
        Typed text
      </div>
      <div className="xterm">
        <span data-testid="row">note</span>
      </div>
    </Page>,
  );
  for (const id of ["editable", "row"]) {
    select(view.getByTestId(id));
    expect(fireEvent.contextMenu(view.getByTestId(id))).toBe(true);
  }
  expect(view.queryByTestId("menu-item-add-to-context")).toBeNull();
});

// `mock.module` outlives this file: put the real module back for the files that run after it.
afterAll(() => {
  mock.module("../../contextBasket.js", () => realBasket);
});

import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { MenuItem } from "../menu.js";
import { useRowContextMenu } from "./useRowContextMenu.js";

afterEach(cleanup);

function Harness({ items }: { items: MenuItem[] }) {
  const { onContextMenu, onKeyDown, menu } = useRowContextMenu(items);
  return (
    <div data-testid="row" tabIndex={0} onContextMenu={onContextMenu} onKeyDown={onKeyDown}>
      row
      {menu}
    </div>
  );
}

function items(run?: () => void): MenuItem[] {
  return [
    { id: "rename", label: "Rename", enabled: true },
    { id: "delete", label: "Delete", enabled: true, run },
  ];
}

test("right-click opens the same menu with the supplied items", () => {
  const { getByTestId, queryByTestId } = render(<Harness items={items()} />);
  expect(queryByTestId("menu-item-rename")).toBeNull();
  fireEvent.contextMenu(getByTestId("row"), { clientX: 40, clientY: 60 });
  expect(getByTestId("menu-item-rename")).toBeTruthy();
  expect(getByTestId("menu-item-delete")).toBeTruthy();
});

test("right-click is cancelled (no native menu) even with no items", () => {
  const { getByTestId, queryByTestId } = render(<Harness items={[]} />);
  // Returns false when the handler called preventDefault.
  const notCancelled = fireEvent.contextMenu(getByTestId("row"));
  expect(notCancelled).toBe(false);
  expect(queryByTestId("menu-item-rename")).toBeNull();
});

test("Shift+F10 opens the menu for keyboard users", () => {
  const { getByTestId } = render(<Harness items={items()} />);
  fireEvent.keyDown(getByTestId("row"), { key: "F10", shiftKey: true });
  expect(getByTestId("menu-item-rename")).toBeTruthy();
});

test("the Menu key opens the menu for keyboard users", () => {
  const { getByTestId } = render(<Harness items={items()} />);
  fireEvent.keyDown(getByTestId("row"), { key: "ContextMenu" });
  expect(getByTestId("menu-item-rename")).toBeTruthy();
});

test("choosing an item runs it and closes the menu", async () => {
  let ran = 0;
  const { getByTestId, queryByTestId } = render(<Harness items={items(() => { ran += 1; })} />);
  fireEvent.contextMenu(getByTestId("row"), { clientX: 10, clientY: 10 });
  fireEvent.click(getByTestId("menu-item-delete"));
  // run + onClose are awaited inside the menu; wait for the close re-render.
  await waitFor(() => expect(queryByTestId("menu-item-delete")).toBeNull());
  expect(ran).toBe(1);
});

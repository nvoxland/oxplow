import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { NarrowRail } from "./NarrowRail.js";

afterEach(cleanup);

const rail = (close: () => void) => (
  <div data-testid="rail-hud">
    <button data-testid="open-page" onClick={close}>page</button>
  </div>
);

test("a wide window shows the rail in place and no button", () => {
  const { queryByLabelText, getByTestId } = render(<NarrowRail narrow={false}>{rail}</NarrowRail>);
  expect(getByTestId("rail-hud")).toBeTruthy();
  expect(queryByLabelText("Show the rail")).toBeNull();
});

test("a narrow window hides the rail until the button shows it", () => {
  const { getByLabelText, queryByTestId } = render(<NarrowRail narrow>{rail}</NarrowRail>);
  expect(queryByTestId("rail-hud")).toBeNull();
  fireEvent.click(getByLabelText("Show the rail"));
  expect(queryByTestId("rail-hud")).not.toBeNull();
});

test("Escape, a click outside, and opening a page hide the overlay", () => {
  const { getByLabelText, getByTestId, queryByTestId } = render(
    <div><span data-testid="outside">x</span><NarrowRail narrow>{rail}</NarrowRail></div>,
  );
  const show = () => fireEvent.click(getByLabelText("Show the rail"));
  show();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(queryByTestId("rail-hud")).toBeNull();
  show();
  fireEvent.mouseDown(getByTestId("rail-hud"));
  expect(queryByTestId("rail-hud")).not.toBeNull();
  fireEvent.mouseDown(getByTestId("outside"));
  expect(queryByTestId("rail-hud")).toBeNull();
  show();
  fireEvent.click(getByTestId("open-page"));
  expect(queryByTestId("rail-hud")).toBeNull();
});

test("widening the window resets the overlay", () => {
  const { getByLabelText, rerender, queryByTestId } = render(<NarrowRail narrow>{rail}</NarrowRail>);
  fireEvent.click(getByLabelText("Show the rail"));
  rerender(<NarrowRail narrow={false}>{rail}</NarrowRail>);
  expect(queryByTestId("rail-hud")).not.toBeNull();
  rerender(<NarrowRail narrow>{rail}</NarrowRail>);
  expect(queryByTestId("rail-hud")).toBeNull();
});

import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { Slideover } from "./Slideover.js";

afterEach(cleanup);

// A field its content focused (a flow's) keeps the focus: moving it to the
// close button would blur the field, and an empty one ends its flow.
test("content that took focus keeps it when the panel opens", () => {
  const view = render(
    <Slideover open onClose={() => {}} title="T" testId="s">
      <input autoFocus data-testid="field" />
    </Slideover>,
  );
  expect(document.activeElement).toBe(view.getByTestId("field"));
});

// Otherwise focus moves inside the panel, so Escape and Tab work there.
test("with nothing focused inside, focus moves to the panel's first control", () => {
  const view = render(
    <Slideover open onClose={() => {}} title="T" testId="s">
      <p>text</p>
    </Slideover>,
  );
  expect(view.getByTestId("s").contains(document.activeElement)).toBe(true);
});

// Escape closes the topmost layer: with a menu open, or a field that took
// the Escape itself, the panel stays.
test("Escape closes the panel, but not while a menu is open or a field took it", () => {
  let closed = 0;
  const view = render(
    <Slideover open onClose={() => closed++} title="T" testId="s">
      <input data-testid="field" onKeyDown={(e) => e.key === "Escape" && e.preventDefault()} />
    </Slideover>,
  );
  fireEvent.keyDown(view.getByTestId("field"), { key: "Escape" });
  expect(closed).toBe(0);
  const menu = document.createElement("div");
  menu.setAttribute("data-oxplow-menu", "");
  document.body.appendChild(menu);
  fireEvent.keyDown(window, { key: "Escape" });
  expect(closed).toBe(0);
  menu.remove();
  fireEvent.keyDown(window, { key: "Escape" });
  expect(closed).toBe(1);
});

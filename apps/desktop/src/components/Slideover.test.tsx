import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

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

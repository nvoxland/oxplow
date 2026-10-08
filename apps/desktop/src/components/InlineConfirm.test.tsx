import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { InlineConfirm } from "./InlineConfirm.js";

afterEach(cleanup);

function renderInRow(onConfirm: () => void) {
  return render(
    <div>
      <div tabIndex={0} data-testid="row">
        <InlineConfirm confirmLabel="Close session" onConfirm={onConfirm} testIdPrefix="c" />
      </div>
      <button type="button" data-testid="elsewhere">
        elsewhere
      </button>
    </div>,
  );
}

/// WebKit doesn't focus a clicked button: focus goes to the focusable
/// element around the pair first. That mustn't revert it, or the click on
/// Confirm is lost.
test("focus moving to an element around the pair keeps it armed", () => {
  let confirmed = 0;
  const { getByTestId } = renderInRow(() => confirmed++);
  fireEvent.click(getByTestId("c-trigger"));
  const confirm = getByTestId("c-confirm");
  fireEvent.blur(confirm, { relatedTarget: getByTestId("row") });
  fireEvent.click(getByTestId("c-confirm"));
  expect(confirmed).toBe(1);
});

/// A press outside the pair, or focus moving to something else, reverts it.
test("a press outside or focus elsewhere reverts it", () => {
  const { getByTestId, queryByTestId } = renderInRow(() => {});
  fireEvent.click(getByTestId("c-trigger"));
  fireEvent.mouseDown(getByTestId("elsewhere"));
  expect(queryByTestId("c-confirm")).toBeNull();
  fireEvent.click(getByTestId("c-trigger"));
  fireEvent.blur(getByTestId("c-confirm"), { relatedTarget: getByTestId("elsewhere") });
  expect(queryByTestId("c-confirm")).toBeNull();
  fireEvent.click(getByTestId("c-trigger"));
  fireEvent.keyDown(window, { key: "Escape" });
  expect(queryByTestId("c-confirm")).toBeNull();
});

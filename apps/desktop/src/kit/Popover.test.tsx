import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { useRef, useState } from "react";

import { Popover } from "./Popover.js";

afterEach(cleanup);

function Host() {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(true);
  return (
    <div>
      <button ref={anchor} type="button" data-testid="anchor" onClick={() => setOpen(true)}>
        When
      </button>
      <button type="button" data-testid="elsewhere">
        Elsewhere
      </button>
      {open ? (
        <Popover anchor={anchor} label="Custom range" onClose={() => setOpen(false)} testId="pop">
          <input data-testid="inside" />
        </Popover>
      ) : null}
    </div>
  );
}

// A popover gives a control more room than its row: a dialog over the
// page, closed by Escape or a click outside, focus back on what opened it.
test("a Popover is a dialog that Escape closes, giving focus back to its anchor", () => {
  const view = render(<Host />);
  const pop = view.getByTestId("pop");
  expect(pop.getAttribute("role")).toBe("dialog");
  expect(pop.getAttribute("aria-label")).toBe("Custom range");
  fireEvent.keyDown(view.getByTestId("inside"), { key: "Escape" });
  expect(view.queryByTestId("pop")).toBeNull();
  expect(document.activeElement).toBe(view.getByTestId("anchor"));
});

test("a click outside closes a Popover; a click inside does not", () => {
  const view = render(<Host />);
  fireEvent.mouseDown(view.getByTestId("inside"));
  expect(view.queryByTestId("pop")).not.toBeNull();
  fireEvent.mouseDown(view.getByTestId("elsewhere"));
  expect(view.queryByTestId("pop")).toBeNull();
});

// Opened from somewhere else (a menu item), it takes focus, so Escape
// reaches it; one whose content took focus keeps it there.
test("a Popover takes focus when it opens, unless something inside already has it", () => {
  const view = render(<Host />);
  expect(document.activeElement).toBe(view.getByTestId("pop"));
  cleanup();
  function Focused() {
    const anchor = useRef<HTMLButtonElement>(null);
    return (
      <div>
        <button ref={anchor} type="button">When</button>
        <Popover anchor={anchor} label="Custom range" onClose={() => {}} testId="pop">
          <input data-testid="inside" autoFocus />
        </Popover>
      </div>
    );
  }
  const focused = render(<Focused />);
  expect(document.activeElement).toBe(focused.getByTestId("inside"));
});

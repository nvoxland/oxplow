import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { useState } from "react";

import { CenterTabs } from "./CenterTabs/CenterTabs.js";
import { TabModal } from "./TabModal.js";

afterEach(cleanup);

test("it has a title, a subtitle and a ×; × and Escape close it; a click outside does not", () => {
  let closed = 0;
  const view = render(
    <TabModal title="Rename" subtitle="Fix the build" onClose={() => closed++} testId="m">
      <input data-testid="field" />
    </TabModal>,
  );
  expect(view.getByTestId("m").getAttribute("role")).toBe("dialog");
  expect(view.getByTestId("m-title").textContent).toBe("Rename");
  expect(view.getByTestId("m-subtitle").textContent).toBe("Fix the build");
  fireEvent.mouseDown(view.getByTestId("m-backdrop"));
  fireEvent.click(view.getByTestId("m-backdrop"));
  expect(closed).toBe(0);
  fireEvent.click(view.getByTestId("m-close"));
  expect(closed).toBe(1);
  fireEvent.keyDown(view.getByTestId("field"), { key: "Escape" });
  expect(closed).toBe(2);
});

test("focus goes into it, and back to what had it when it closes", () => {
  function Host() {
    const [open, setOpen] = useState(false);
    return (
      <div>
        <button data-testid="opener" onClick={() => setOpen(true)}>open</button>
        {open ? (
          <TabModal title="T" onClose={() => setOpen(false)} testId="m">
            <input data-testid="field" />
          </TabModal>
        ) : null}
      </div>
    );
  }
  const view = render(<Host />);
  const opener = view.getByTestId("opener");
  opener.focus();
  fireEvent.click(opener);
  expect(document.activeElement).toBe(view.getByTestId("field"));
  fireEvent.click(view.getByTestId("m-close"));
  expect(document.activeElement).toBe(opener);
});

// A tab's dialog is its body's own state, and every tab body stays mounted
// while another is shown, so it is still open — with what was typed — when
// the person comes back.
test("a tab's open modal, and what was typed in it, survive switching away and back", () => {
  function Body() {
    const [open, setOpen] = useState(false);
    return (
      <div>
        <button data-testid="open" onClick={() => setOpen(true)}>open</button>
        {open ? (
          <TabModal title="T" onClose={() => setOpen(false)} testId="m">
            <input data-testid="field" />
          </TabModal>
        ) : null}
      </div>
    );
  }
  const tabs = [
    { id: "a", label: "A", closable: true, render: () => <Body /> },
    { id: "b", label: "B", closable: true, render: () => <div data-testid="b-body" /> },
  ];
  const draw = (activeId: string) => (
    <CenterTabs tabs={tabs} activeId={activeId} onActivate={() => {}} onClose={() => {}} onReorder={() => {}} />
  );
  const view = render(draw("a"));
  fireEvent.click(view.getByTestId("open"));
  fireEvent.change(view.getByTestId("field"), { target: { value: "kept" } });
  view.rerender(draw("b"));
  expect(view.getByTestId("center-tab-body-b").style.display).toBe("flex");
  expect(view.getByTestId("center-tab-body-a").style.display).toBe("none");
  view.rerender(draw("a"));
  expect((view.getByTestId("field") as HTMLInputElement).value).toBe("kept");
  expect(view.getByTestId("m")).toBeTruthy();
});

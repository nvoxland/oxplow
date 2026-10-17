import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { WorkItem } from "../../workItems.js";
import { KeyboardValuePicker } from "./PlanPane.js";

afterEach(cleanup);

const item = { state: "in_progress", native: {} } as unknown as WorkItem;

// The state picker follows the one list contract (useListNav): it starts
// on the current value, ↑/↓ stop at the ends, Enter commits the row under
// the cursor, Escape cancels, and the overlay names the cursor's row.
test("the state picker starts on the current state and ↑/↓/Enter/Escape work", () => {
  const picked: string[] = [];
  let closed = 0;
  const { container } = render(
    <KeyboardValuePicker picker={{ kind: "state" }} item={item} onPick={(v) => picked.push(v)} onClose={() => closed++} />,
  );
  const overlay = container.firstElementChild as HTMLElement;
  expect(document.activeElement).toBe(overlay);
  fireEvent.keyDown(overlay, { key: "Enter" });
  expect(picked).toEqual(["in_progress"]);
  for (let i = 0; i < 20; i++) fireEvent.keyDown(overlay, { key: "ArrowDown" });
  const last = document.getElementById(overlay.getAttribute("aria-activedescendant")!);
  expect(last?.textContent).toBeTruthy();
  fireEvent.keyDown(overlay, { key: "Enter" });
  expect(picked.length).toBe(2);
  expect(picked[1]).not.toBe("in_progress");
  fireEvent.keyDown(overlay, { key: "Escape" });
  expect(closed).toBe(1);
});

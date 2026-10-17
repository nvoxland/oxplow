import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { MetricCatalogEntry } from "../../api.js";
import { MetricPickerPanel } from "./MetricPickerPanel.js";

afterEach(cleanup);

const entry = (key: string, title: string): MetricCatalogEntry => ({
  key,
  title,
  kind: "gauge",
  language: null,
  scope: "built-in",
  enabled: true,
  target: null,
  trigger: "onCapture",
  toggleable: false,
  category: "operational",
});
const catalog = [entry("m.a", "Alpha"), entry("m.b", "Beta"), entry("m.c", "Gamma")];

function panel() {
  const picked: string[] = [];
  let closed = 0;
  const view = render(
    <MetricPickerPanel
      catalog={catalog}
      anchor={{ x: 10, y: 10 }}
      addedKeys={new Set()}
      onPick={(k) => picked.push(k)}
      onClose={() => closed++}
    />,
  );
  return { ...view, picked, closed: () => closed };
}

// The picker follows the one list contract (useListNav): ↑/↓ move and stop at
// the ends, Enter picks the row under the cursor, Escape closes, and the
// input names the cursor's row so focus never leaves it.
test("↑/↓ move through the rows and stop at the ends, Enter picks, Escape closes", () => {
  const { getByTestId, picked, closed } = panel();
  const input = getByTestId("metric-picker-search");
  fireEvent.keyDown(input, { key: "ArrowUp" });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(picked).toEqual(["m.a"]);
  for (let i = 0; i < 5; i++) fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(picked).toEqual(["m.a", "m.c"]);
  fireEvent.keyDown(input, { key: "Escape" });
  expect(closed()).toBe(1);
});

test("the input names the row under the cursor", () => {
  const { getByTestId } = panel();
  const input = getByTestId("metric-picker-search");
  fireEvent.keyDown(input, { key: "ArrowDown" });
  const active = input.getAttribute("aria-activedescendant");
  expect(active).not.toBeNull();
  expect(document.getElementById(active!)?.getAttribute("data-testid")).toBe("metric-picker-row-m.b");
});

import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import { CustomComponentViz } from "./CustomComponentViz.js";

afterEach(cleanup);

const run = (custom: LensRun["lens"]["custom"]) =>
  ({
    lens: { id: "x/burn", extension: "x", title: "Burn", custom },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  }) as unknown as LensRun;

const fallback = <div data-testid="the-table">table</div>;

// P6b.D4: the frame is sandboxed to scripts only, at its bundle's URL,
// marked custom; with no component, or no daemon, the table shows.
test("the component's frame is scripts-only, at its bundle, marked custom", () => {
  const view = render(
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId="str1" fallback={fallback} base="http://127.0.0.1:9" />,
  );
  const frame = view.getByTestId("custom-component-frame");
  expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
  expect(frame.getAttribute("src")).toBe("http://127.0.0.1:9/components/str1/x/burndown/");
  expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
  expect(view.getByTestId("custom-component-badge").textContent).toBe("custom");
  expect(view.queryByTestId("the-table")).toBeNull();
});

test("no component or no daemon shows the table", () => {
  const a = render(<CustomComponentViz run={run(null)} streamId={null} fallback={fallback} base="http://127.0.0.1:9" />);
  expect(a.getByTestId("the-table")).toBeTruthy();
  cleanup();
  const b = render(<CustomComponentViz run={run({ component: "burndown", props: null })} streamId={null} fallback={fallback} base={null} />);
  expect(b.getByTestId("the-table")).toBeTruthy();
});

test("a frame that never says ready, or navigates away, falls back to the table", async () => {
  const slow = render(
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId={null} fallback={fallback} base="http://127.0.0.1:9" readyTimeoutMs={10} />,
  );
  fireEvent.load(slow.getByTestId("custom-component-frame"));
  await waitFor(() => expect(slow.getByTestId("custom-component-fallback").textContent).toContain("didn't start"));
  expect(slow.getByTestId("the-table")).toBeTruthy();
  cleanup();
  const away = render(
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId={null} fallback={fallback} base="http://127.0.0.1:9" />,
  );
  fireEvent.load(away.getByTestId("custom-component-frame"));
  fireEvent.load(away.getByTestId("custom-component-frame"));
  await waitFor(() => expect(away.getByTestId("custom-component-fallback").textContent).toContain("navigated away"));
});

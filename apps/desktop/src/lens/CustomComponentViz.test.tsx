import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { LensRun } from "../tauri-bridge/generated/bindings.js";

// tsk984: the host loads the component's bundle (its version) before it
// shows a frame, and the frame is served that version.
const realApi = await import("../api.js");
const loads: string[] = [];
let loadFails: string | null = null;
mock.module("../api.js", () => ({
  ...realApi,
  loadComponent: async (id: string) => {
    loads.push(id);
    if (loadFails) throw new Error(loadFails);
    return "v-1";
  },
}));

const { CustomComponentViz } = await import("./CustomComponentViz.js");

afterEach(() => {
  cleanup();
  loads.length = 0;
  loadFails = null;
});

const run = (custom: LensRun["lens"]["custom"]) =>
  ({
    lens: { id: "x/burn", extension: "x", title: "Burn", custom },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  }) as unknown as LensRun;

const fallback = <div data-testid="the-table">table</div>;

// P6b.D4: the frame is sandboxed to scripts only, at its loaded bundle's
// URL, marked custom; with no component, or no daemon, the table shows.
test("the component's frame is scripts-only, at its loaded bundle, marked custom", async () => {
  const view = render(
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId="str1" fallback={fallback} base="http://127.0.0.1:9" />,
  );
  const frame = await view.findByTestId("custom-component-frame");
  expect(loads).toEqual(["x/burn"]);
  expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
  expect(frame.getAttribute("src")).toBe("http://127.0.0.1:9/components/v/v-1/");
  expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
  expect(view.getByTestId("custom-component-badge").textContent).toBe("custom");
  expect(view.queryByTestId("the-table")).toBeNull();
});

test("no component or no daemon shows the table, loading nothing", () => {
  const a = render(<CustomComponentViz run={run(null)} streamId={null} fallback={fallback} base="http://127.0.0.1:9" />);
  expect(a.getByTestId("the-table")).toBeTruthy();
  cleanup();
  const b = render(<CustomComponentViz run={run({ component: "burndown", props: null })} streamId={null} fallback={fallback} base={null} />);
  expect(b.getByTestId("the-table")).toBeTruthy();
  expect(loads).toEqual([]);
});

test("a bundle that can't be loaded shows the table and why", async () => {
  loadFails = "`x/c`: no `index.html`";
  const view = render(
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId={null} fallback={fallback} base="http://127.0.0.1:9" />,
  );
  await waitFor(() => expect(view.getByTestId("custom-component-fallback").textContent).toContain("no `index.html`"));
  expect(view.getByTestId("the-table")).toBeTruthy();
  expect(view.queryByTestId("custom-component-frame")).toBeNull();
});

test("a frame that never says ready, or navigates away, falls back to the table", async () => {
  const slow = render(
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId={null} fallback={fallback} base="http://127.0.0.1:9" readyTimeoutMs={10} />,
  );
  fireEvent.load(await slow.findByTestId("custom-component-frame"));
  await waitFor(() => expect(slow.getByTestId("custom-component-fallback").textContent).toContain("didn't start"));
  expect(slow.getByTestId("the-table")).toBeTruthy();
  cleanup();
  const away = render(
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId={null} fallback={fallback} base="http://127.0.0.1:9" />,
  );
  const frame = await away.findByTestId("custom-component-frame");
  fireEvent.load(frame);
  fireEvent.load(frame);
  await waitFor(() => expect(away.getByTestId("custom-component-fallback").textContent).toContain("navigated away"));
});

// P9.A1: a replacement's lens says what stands in for a component that
// can't be shown — the core component, not this lens's table.
test("with `onFailure`, a component that can't be shown is reported and shows nothing itself", async () => {
  const reasons: string[] = [];
  const slow = render(
    <CustomComponentViz
      run={run({ component: "burndown", props: null })}
      streamId={null}
      fallback={fallback}
      onFailure={(reason) => reasons.push(reason)}
      base="http://127.0.0.1:9"
      readyTimeoutMs={10}
    />,
  );
  fireEvent.load(await slow.findByTestId("custom-component-frame"));
  await waitFor(() => expect(reasons).toEqual(["The component didn't start."]));
  // The caller shows what stands in (a replacement: oxplow's own, outside).
  expect(slow.queryByTestId("custom-component-frame")).toBeNull();
  expect(slow.queryByTestId("the-table")).toBeNull();
  expect(slow.queryByTestId("custom-component-fallback")).toBeNull();
  cleanup();
  reasons.length = 0;
  const hostless = render(
    <CustomComponentViz
      run={run({ component: "burndown", props: null })}
      streamId={null}
      fallback={fallback}
      onFailure={(reason) => reasons.push(reason)}
      base={null}
    />,
  );
  await waitFor(() => expect(reasons).toEqual(["Its component can't be shown here."]));
  expect(hostless.queryByTestId("the-table")).toBeNull();
});

// The bundle is the main worktree's whatever the stream: a stream switch
// keeps the frame and loads nothing again.
test("a stream switch keeps the same frame", async () => {
  const props = (streamId: string) => (
    <CustomComponentViz run={run({ component: "burndown", props: null })} streamId={streamId} fallback={fallback} base="http://127.0.0.1:9" />
  );
  const view = render(props("str1"));
  const frame = await view.findByTestId("custom-component-frame");
  view.rerender(props("str2"));
  expect(view.getByTestId("custom-component-frame")).toBe(frame);
  expect(loads).toEqual(["x/burn"]);
});

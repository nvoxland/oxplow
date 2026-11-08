import { RunnerContext } from "../commands/RunnerContext.js";
import { recordingRunner } from "../commands/testing.js";
import { afterEach, expect, test } from "bun:test";
import { act, cleanup, createEvent, fireEvent, render, waitFor } from "@testing-library/react";
import { parse } from "yaml";

import { DashboardEditContext } from "./dashboardAuthoring.js";
import { DashboardGrid } from "./DashboardGrid.js";

afterEach(cleanup);

const yaml = `# Keep the dashboard note
title: Build health
category: Dashboards
view:
  grid:
    - id: tests
      title: Tests # Keep the tile note
      embed: { page: acme/tests }
    - id: coverage
      embed: { page: acme/coverage }
`;
const tiles = [
  { id: "tests", size: "small" as const, body: <span>Test results</span> },
  { id: "coverage", size: "wide" as const, body: <span>Coverage results</span> },
];

function fixture({ writable = true, write = async (_input: { spec: string }): Promise<unknown> => {} } = {}) {
  const writes: { extension: string; page: string; spec: string; deploy: true }[] = [];
  const errors: { label: string; message: string }[] = [];
  const spec = { page: "my-pages/build-health", extension: "my-pages", slug: "build-health", yaml, writable };
  let reads = 0;
  const view = render(<RunnerContext.Provider value={recordingRunner((e) => errors.push({ label: e.label, message: e.message ?? "" }))}>
    <DashboardEditContext.Provider value={{
      read: async () => { reads += 1; return spec; },
      write: async (input) => { writes.push(input); return write(input); },
    }}><DashboardGrid page={spec.page} tiles={tiles} /></DashboardEditContext.Provider>
  </RunnerContext.Provider>);
  const order = () => Array.from(view.getByTestId("dashboard-grid").children)
    .map((element) => element.getAttribute("data-testid"));
  const ready = async () => {
    await waitFor(() => expect(reads).toBeGreaterThan(0));
    await waitFor(() => expect(view.getByTestId("dashboard-tile-tests").getAttribute("draggable")).toBe(String(writable)));
  };
  const remove = () => {
    fireEvent.contextMenu(view.getByTestId("dashboard-tile-tests"), { clientX: 20, clientY: 20 });
    fireEvent.click(view.getByText("Remove tile…"));
  };
  return { view, writes, errors, spec, order, ready, remove };
}

function drag(from: HTMLElement, to: HTMLElement) {
  const data: Record<string, string> = {};
  const dataTransfer = {
    setData: (key: string, value: string) => { data[key] = value; },
    getData: (key: string) => data[key] ?? "",
    get types() { return Object.keys(data); },
    effectAllowed: "",
    dropEffect: "",
  };
  fireEvent.dragStart(from, { dataTransfer });
  for (const make of [createEvent.dragOver, createEvent.drop]) {
    fireEvent(to, make(to, { dataTransfer }));
  }
}

test("dragging a writable dashboard tile saves the new order and preserves current YAML comments", async () => {
  const f = fixture();
  await f.ready();
  f.spec.yaml += "# An undeployed edit\n";
  drag(f.view.getByTestId("dashboard-tile-coverage"), f.view.getByTestId("dashboard-tile-tests"));
  await waitFor(() => expect(f.writes.length).toBe(1));
  expect(f.order()).toEqual(["dashboard-tile-coverage", "dashboard-tile-tests"]);
  expect(f.writes[0]).toMatchObject({ extension: "my-pages", page: "build-health", deploy: true });
  expect(parse(f.writes[0].spec).view.grid.map((tile: { id: string }) => tile.id)).toEqual(["coverage", "tests"]);
  expect(f.writes[0].spec).toContain("# Keep the dashboard note");
  expect(f.writes[0].spec).toContain("# Keep the tile note");
  expect(f.writes[0].spec).toContain("# An undeployed edit");
  expect(f.errors).toEqual([]);
});

test("read-only dashboards expose no drag or remove menu", async () => {
  const f = fixture({ writable: false });
  await f.ready();
  fireEvent.contextMenu(f.view.getByTestId("dashboard-tile-tests"));
  expect(f.view.queryByText("Remove tile…") === null).toBe(true);
  drag(f.view.getByTestId("dashboard-tile-coverage"), f.view.getByTestId("dashboard-tile-tests"));
  expect(f.writes).toEqual([]);
  expect(f.order()).toEqual(["dashboard-tile-tests", "dashboard-tile-coverage"]);
});

test("right-click removal asks inline: Cancel and Escape keep the tile; Confirm saves removal", async () => {
  const f = fixture();
  await f.ready();
  f.remove();
  expect(f.view.getByText("Remove this tile?")).toBeTruthy();
  expect(f.writes).toEqual([]);
  fireEvent.click(f.view.getByRole("button", { name: "Cancel" }));
  expect(f.view.queryByText("Remove this tile?") === null).toBe(true);
  expect(f.order()).toEqual(["dashboard-tile-tests", "dashboard-tile-coverage"]);
  f.remove();
  fireEvent.keyDown(f.view.getByRole("button", { name: "Confirm" }), { key: "Escape" });
  expect(f.view.queryByText("Remove this tile?") === null).toBe(true);
  expect(f.writes).toEqual([]);
  f.remove();
  fireEvent.click(f.view.getByRole("button", { name: "Confirm" }));
  await waitFor(() => expect(f.writes.length).toBe(1));
  expect(parse(f.writes[0].spec).view.grid.map((tile: { id: string }) => tile.id)).toEqual(["coverage"]);
  expect(f.order()).toEqual(["dashboard-tile-coverage"]);
  expect(f.writes[0].spec).toContain("# Keep the dashboard note");
});

test("a failed save restores the optimistic drag order and records the error", async () => {
  let reject!: (reason: Error) => void;
  const pending = new Promise<void>((_resolve, fail) => { reject = fail; });
  const f = fixture({ write: async () => pending });
  await f.ready();
  drag(f.view.getByTestId("dashboard-tile-coverage"), f.view.getByTestId("dashboard-tile-tests"));
  await waitFor(() => expect(f.writes.length).toBe(1));
  expect(f.order()).toEqual(["dashboard-tile-coverage", "dashboard-tile-tests"]);
  await act(async () => { reject(new Error("Page changed before save")); await pending.catch(() => {}); });
  await waitFor(() => expect(f.errors.length).toBe(1));
  expect(f.errors[0]).toMatchObject({ label: "Edit dashboard" });
  expect(f.errors[0].message).toContain("Page changed before save");
  expect(f.order()).toEqual(["dashboard-tile-tests", "dashboard-tile-coverage"]);
});

// A new dashboard has no tiles: it says how to add one, not a blank page.
test("a dashboard with no tiles says how to add one", () => {
  const view = render(<DashboardEditContext.Provider value={{
    read: async () => ({ page: "my-pages/empty", extension: "my-pages", slug: "empty", yaml: "title: Empty\nview:\n  grid: []\n", writable: true }),
    write: async () => {},
  }}><DashboardGrid page="my-pages/empty" tiles={[]} /></DashboardEditContext.Provider>);
  expect(view.getByTestId("dashboard-empty").textContent).toContain("Pin to Dashboard");
});
